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

Evaluate each role independently. Interactive member policies are eligible under the runner contract; automatic review or fixed allow/deny capability may remain available without being the selected mode. Preserve native guards, managed policy, explicit denies and sandbox boundaries. A backend with unknown detector/composer coverage permits manual terminal work and holds generated input.

Goal support is a host capability; preserve supplied activation, pause, completion and budget authority. A verified native receive adapter is evaluated separately: a host satisfying both Director prerequisites may use explicit manual `poll` or finite `wait` when its native adapter is unavailable. Background execution alone establishes no unsolicited wake or per-record reception guarantee.

## Spawn argv {#spawn-argv}

These are the child argv with omitted permission selection. The multiplexer starts the CAFleet PTY runner, which preserves the literal child argv, cwd and required environment. Explicit selection changes only the native permission argument through the [launch-policy contract](member-runtime.md#launch-policy-and-capability-registry).

| Backend | Spawn argv |
|---|---|
| `claude` | `claude --permission-mode dontAsk --prompt-suggestions false --allowedTools <rule>... --name <member-name> <prompt>` |
| `codex` | `codex --ask-for-approval never --sandbox workspace-write <prompt>` |
| `opencode` | `opencode mini --standalone --agent cafleet --prompt <prompt>` |

Shared contract:

- Each omitted policy retains its existing command posture; an explicit interactive policy may display native approval prompts.
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
| `claude` | none | yes, via `--name <member-name>` | Runs the broker commands under its spawn-time allow rules, read-only commands, and every command matching the operator's `permissions.allow`; a denied command routes to the Director | A permission profile in `~/.claude/settings.json` — a permission posture, not a spawn dependency ([Claude](#claude)) |
| `codex` | kernel-enforced ([Codex](#codex)) | no — locate the pane through `cafleet member list` (`pane_id` is ground truth) | Runs commands its workspace/host rules permit; routes required denied commands | `~/.codex/rules/cafleet.rules` (`CODEX_HOME` relocates it), plus the `~/.codex/config.toml` settings and a trusted working directory — a permission posture, not a spawn dependency ([the rules file](#cafleet-rules-file)) |
| `opencode` | none | no — locate the pane through `cafleet member list` | Deny-by-default allowlist; everything outside it routes to the Director | `~/.opencode/agents/cafleet.md` (`OPENCODE_CONFIG_DIR` relocates it) — a spawn precondition ([the agent preset](#cafleet-agent-preset)) |

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

`--permission-mode dontAsk` is the omitted Claude member policy. Explicit native modes retain their own permission behavior. Under `dontAsk`, a member runs these calls without a prompt: its
[spawn-time allow rules](#spawn-time-allow-rules), Claude Code's built-in
read-only commands, file reads inside its working directory, and every tool
call matching `permissions.allow`. Every other call — a shell command or a
file edit, including one matching `permissions.ask` — is denied without a
prompt. A write to one of Claude Code's protected paths, such as a project's
`.git` or `.claude` directory, is denied whatever the allow list holds. Beyond
the broker commands, the operator's settings are therefore the member's allow
list; the recommended profiles are in
[Claude Code configuration](../quickstart.md#claude-code).

Launch-policy evidence is pinned to Claude Code 2.1.302 and Codex CLI 0.163.0. The inspected Claude parser accepts canonical `default` and alias `manual`, although help renders the canonical mode as `manual`; explicit argv preserves the selected spelling. [Claude’s permission-mode reference](https://code.claude.com/docs/en/permission-modes) documents the alias. Codex help confirms `on-request` and `never` with the explicit workspace sandbox retained. These observations establish launch-policy eligibility; detector/composer coverage remains unverified until the exact-version acceptance gates pass.

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
| `Edit`, `Write` | Both | Every member that writes files | A member's file edit is denied unless an entry matches it. No entry covers a protected path. |
| `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)` under `ask` | Both | The Director | The operator confirms each dispatch; a member's own attempt is denied. |

The auto mode profile carries no `Bash(cafleet ...)` allow entry: the
classifier reviews each Director command that no rule matches, auto mode
approves the Director's file edits inside the working directory, and members
receive their broker commands as spawn-time allow rules.

`cafleet member prompt` and `cafleet member exec` stay under `ask` in both
profiles because the first keystrokes arbitrary text into a member's pane and
the second runs an arbitrary command there. An ask rule outranks every allow
rule, so the entries hold beside `Bash(cafleet *)`.

`Edit` and `Write` apply to every session that reads the settings file, so
they also pre-approve file edits in your own sessions. To confine the
approval, replace both with a path rule such as `Edit(~/work/**)`, which
covers every file-editing tool. No entry reaches a protected path: a member's
edit under a project's `.git` or `.claude` directory is always denied, and the
Director session makes that edit instead. When `CLAUDE_CONFIG_DIR` relocates
the config directory, write the `Read` entry against that directory's
`skills/**`.

### What members can run {#claude-member-permissions}

A member whose shell command is denied asks its Director to run it. The
Director runs it with [`cafleet member exec`](cli-options.md#member-exec),
which is under `ask`, so each routed command costs the operator one
confirmation. Add an allow entry for every project command members run as
part of their work — a test or lint task, for example, as `Bash(npm test *)` —
so those commands run without routing. Routing carries shell commands only: a
denied file edit needs a matching `Edit` entry, or, on a protected path, the
Director to make the edit.

| Symptom | Cause | Resolution |
|---|---|---|
| A member reports a denied shell command | The command is outside the spawn-time allow rules, the read-only set, and the allow list | Add an allow entry for the command. Until then, each Director dispatch of it asks the operator to confirm. |
| A member reports a denied file edit | No `Edit` entry matches the file's path, or the path is protected | Add `Edit` and `Write`, or widen the `Edit` path rule to cover the member's working directory. For a protected path, the Director makes the edit. |
| The Director is prompted for every `cafleet` command | The session runs in a non-auto mode with the auto mode profile | Switch the session to auto mode, or use the other-modes profile |
| The classifier blocks a Director `cafleet` command | Auto mode judged the action outside the request | Add a pattern for that subcommand per [`permissions.allow` coverage](cli-options.md#permissionsallow-coverage) |
| `defaultMode: "auto"` has no effect | It is set in a project or local settings file, or auto mode is unavailable for the organization or model | Move it to `~/.claude/settings.json`, removing it from the project file. When auto mode is unavailable, use the other-modes profile. |

Verify native Monitor authorization separately from Bash broker rules. Monitor events remain broker events during an open user decision. Actual expiry, cancellation, child cleanup and independently observed continuation preservation require acceptance evidence.

## Codex {#codex}

The objective belongs to the thread, with model-callable lifecycle tools and optional token-budget accounting. Retain one native execution session handle for Director inbox reception and explicitly read it while delegated work remains, independently of whether a host goal exists. The installed CAFleet skill's Codex native instructions own tool calls, bounded reads, leases and recovery; the [manual lifecycle](../how-to/mixed-backend-team.md#manual-lifecycle) explains operation. Preserve supplied goal activation, pause, completion and budget authority. A background process alone supplies no unsolicited per-line wake after a model turn ends; fully idle Codex reconciles its inbox on the next user/native activation.

A native shell probe displayed the actual command, exit code, duration and output. Installed protocol metadata describes unsandboxed dispatch while public CLI guidance describes current approval/sandbox settings; the TUI's effective boundary and protocol path were not established. Treat native input as a Director-authorized action rather than assume a second member-tool permission check.

`--sandbox workspace-write` confines writes to the workspace under a
kernel-enforced sandbox — codex is the only backend with one.
`--ask-for-approval never` disables interactive approval prompts (upstream
write-up: <https://developers.openai.com/codex/agent-approvals-security>).

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
```

Codex applies the strictest decision when more than one rule matches
(`forbidden` > `prompt` > `allow`): `cafleet member prompt` and
`cafleet member exec` each match the broad allow and their own rule, so the
`prompt` wins and each invocation keeps requiring approval, while every other
subcommand matches only the broad `["cafleet"]` allow — for every fleet,
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
```

The `allow` rules cover the broker commands members run themselves — the same
commands a claude member receives as
[spawn-time allow rules](#spawn-time-allow-rules); the `prompt` rules keep
`cafleet member prompt` and `cafleet member exec` under operator approval.

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

### The `cafleet` agent preset {#cafleet-agent-preset}

`--agent cafleet` binds the member to `~/.opencode/agents/cafleet.md`
(`OPENCODE_CONFIG_DIR` relocates the `~/.opencode` base — see
[Config-dir resolution](cli-options.md#config-dir-resolution); the spawn
precondition checks the same resolved path `setup` installs to). The
preset is embedded in the released binary (`presets/opencode/cafleet.md`).
`cafleet setup` installs it offline, replacing any existing copy; rerun setup
after upgrading CAFleet to refresh it. The
preset is a spawn precondition: the spawn argv references `--agent cafleet`,
so `cafleet member create --coding-agent opencode` fails with `opencode agent
preset not found at <preset>; run 'cafleet setup --coding-agent opencode'
first` when the file is missing.

Preset discovery includes the relocated directory and default home configuration.
Agent definitions with the same name can merge across those sources; inspect the
effective configuration when customizing a preset.

The preset's `shell` ruleset is deny-by-default: a `"*": "deny"` base first,
then an explicit allowlist translated from the operator's Claude Code
`permissions.allow` set (`cafleet *`, its declared Git commands,
file-inspection utilities, and the project's cargo-backed mise tasks). opencode selects the
**last** matching rule, so this order is the safety floor — every check
resolves to `allow` or `deny`, never `ask`. Explicit `cafleet member prompt *` and `cafleet member exec *` deny
entries follow the `cafleet *` allow, so they win: the preset configures
members, and dispatching a command into another pane is the Director's act. A member's denied command receives no permission prompt, while permitted broker commands run normally. Tool permission and assignment authority are separate: the assigned role and Director instructions determine which allowed operations a member may perform. A permission popup in an opencode pane is
therefore a regression escape, not a runtime decision: capture the pane,
escalate, and extend the allowlist by operator decision — do not answer the
popup ad hoc.

### Safety-floor caveats {#safety-floor-caveats}

The posture is a deny-by-default allowlist, with no OS-level sandbox.
Standalone un-enumerated commands fall to the `"*": "deny"` base. Apply these
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
