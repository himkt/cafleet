# Coding-agent backends

Every member pane runs a coding-agent binary selected from the backend registry. The backend is recorded
per member in `member_placements.coding_agent`; selection and inheritance via
`--coding-agent`, mixed-backend teams, and identity delivery are covered in
[Coding agents](../concepts/coding-agents.md). This page specifies each
backend's spawn argv, selected permission policy, model-flag format, and
version/config requirements.

## Spawn argv {#spawn-argv}

| Backend | Spawn argv |
|---|---|
| `claude` | `claude --permission-mode auto --prompt-suggestions false --allowedTools <rule>... --name <member-name> <prompt>` |
| `codex` | `codex --approve-for-me --sandbox workspace-write <prompt>` |
| `opencode` | `opencode mini --standalone --agent cafleet --prompt <prompt>` |

Shared contract:

- Every member-create path prepends the mandatory command-routing instruction and literal broker identities to the rendered assignment. Members execute task commands under the backend's selected permission policy and route observed permission denials with command/cwd/environment/refusal evidence. Native guards, explicit denies, managed policies and host permission/sandbox settings remain authoritative. Surface unresolved approval to the Director under every policy. When automatic review is selected, surface unavailable automatic mode to the Director. Ordinary nonzero results follow normal recovery. Native file tools remain within role scope.
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

| Backend | OS-level sandbox | Sets the pane title | Native command-permission posture | Preset / config prerequisite |
|---|---|---|---|---|
| `claude` | none | yes, via `--name <member-name>` | Native `auto` review with spawn-time broker rules and existing operator rules | A permission profile in `~/.claude/settings.json` — a permission posture, not a spawn dependency ([Claude](#claude)) |
| `codex` | kernel-enforced ([Codex](#codex)) | no — locate the pane through `cafleet member list` (`pane_id` is ground truth) | Automatic approval review under `--approve-for-me`, retaining the explicit workspace sandbox and host policy | `~/.codex/rules/cafleet.rules` (`CODEX_HOME` relocates it), plus the `~/.codex/config.toml` settings and a trusted working directory — a permission posture, not a spawn dependency ([the rules file](#cafleet-rules-file)) |
| `opencode` | none | no — locate the pane through `cafleet member list` | The preset's shell allowlist over a deny base, with explicit native-dispatch denies | `~/.opencode/agents/cafleet.md` (`OPENCODE_CONFIG_DIR` relocates it) — a spawn precondition ([the agent preset](#cafleet-agent-preset)) |

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

A separate evaluator checks the session condition against the transcript; native background work defers evaluation and returns a host result. Use one native Monitor task for Director inbox records, retaining its handle across separate events and renewing after observed source-task exit. The installed CAFleet skill's Claude native instructions own discovery, invocation, renewal and evidence limits; the [manual lifecycle](../how-to/mixed-backend-team.md#manual-lifecycle) explains operation. Preserve supplied `/goal` state and pending user decisions; manual finite reception remains available when Monitor is unavailable or denied.

Native shell probes displayed command/stdout/stderr followed by a model turn; the trigger and any second permission check were not established. The Director authorizes each routed command under its host rules.

`--permission-mode auto` selects native automatic approval review. The
[spawn-time allow rules](#spawn-time-allow-rules) retain broker access, while
native deny/ask rules, hooks and managed policy retain precedence. Auto mode
can start in Manual when unavailable; explicit asks and hook-forced prompts
can still require a response. Surface actual unavailable mode or unresolved
approval to the Director rather than assuming that the launch flag establishes
automatic execution. A returned permission refusal follows the shared route;
ordinary command errors follow normal debugging. Protected-path and file-tool
rules remain authoritative. The recommended operator profiles are in
[Claude Code configuration](../quickstart.md#claude-code), and the upstream
[permission modes](https://code.claude.com/docs/en/permission-modes) describe
availability and native decisions. No live availability result follows from
CAFleet's argv and prompt tests.

### Spawn-time allow rules {#spawn-time-allow-rules}

cafleet passes the broker commands a claude member needs as session allow
rules on its spawn command line, so the member does not depend on any
`Bash(cafleet ...)` rule in the user's `settings.json`:

| Member | Rules |
|---|---|
| Broker capability | `Bash(cafleet message *)`, including finite `message wait` and `message listen` |
| File and task-command capability | Native auto review and existing host-configured read/edit/command rules, with observed-denial routing |

`--allowedTools` takes several values, so it sits before `--name`, which ends
its value list and keeps the prompt positional. The rules are fixed at spawn:
a later loss of allow rules in the user's settings does not affect a running
member.

The flags do not cover these cases:

- An effective `deny` rule at any settings level, including managed policy,
  refuses the call. An effective `ask` rule retains a human approval boundary
  in auto mode. Both can prevent unattended broker progress, and
  [`cafleet doctor`](cli-options.md#member-permissions) reports them before spawn.
- The managed setting `allowManagedPermissionRulesOnly` makes Claude Code
  ignore `--allowedTools`. `cafleet doctor` reports it.
- A blocking `PreToolUse` hook stops a tool call before permission rules are
  evaluated. `cafleet doctor` cannot evaluate a hook.
- Inspect overdue member readiness/replies directly; settings can change after spawn.
- Task commands and file tools retain native permissions; a returned denial
  routes with its actual evidence. An authorized
  [`cafleet member exec`](cli-options.md#member-exec) retains the Director's
  existing host rules and operator authority.

### What each profile entry is for {#claude-profile-entries}

| Entry | Profile | Needed by | Why |
|---|---|---|---|
| `Bash(cafleet *)` | Other modes | The Director | One pattern pre-approves every subcommand the Director runs. |
| `Skill(cafleet)`, `Skill(cafleet-design-doc)` | Both | The Director and every member | Each loads the installed skills through the Skill tool. |
| `Read(~/.claude/skills/**)` | Both | The Director and every member | Role and reference pages of an installed skill sit outside the working directory. |
| `Edit`, `Write` | Both | Every member that writes files | Pre-approves file edits within the entry's path boundary. Unmatched calls retain native review, and protected paths retain their rules. |
| `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)` under `ask` | Both | The Director | The operator confirms each native dispatch under the existing host policy. |

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
covers every file-editing tool. Protected paths, including a project's `.git`
or `.claude` directory, retain their native rules. Route an actual refusal
for Director review under existing authority. When `CLAUDE_CONFIG_DIR` relocates
the config directory, write the `Read` entry against that directory's
`skills/**`.

### Member command routing {#claude-member-permissions}

A member executes role-authorized task commands using native auto review and
routes an observed permission denial with its exact command, working
directory, required environment and actual refusal. The Director reviews the
evidence under existing host and operator authority; routing grants no override
of native or managed denial. An authorized
[`cafleet member exec`](cli-options.md#member-exec) retains the existing `ask`
rule and operator confirmation. Members use authorized broker communications
and native file tools within role scope. Prerequisites use native readers or
permitted shell reads; an actual denial follows the same route before ready.
Ordinary nonzero results follow normal recovery. Surface unavailable auto or
unresolved approval to the Director, whose supervision handles prompts that
prevent a member from sending. This behavioral contract provides no
deterministic refusal-event interception.

| Symptom | Cause | Resolution |
|---|---|---|
| A member reports a denied task command | Native permissions or automatic review returned a refusal | Review exact command/cwd/environment/refusal evidence under existing host/operator authority, preserve effective denies, and inspect any authorized result. |
| A member reports a denied file edit | Native review, a file rule or protected-path policy refused it | Review the actual refusal and path under existing authority; operator-managed file rules and an authorized Director edit retain their native boundaries. |
| The Director is prompted for every `cafleet` command | The session runs in a non-auto mode with the auto mode profile | Switch the session to auto mode, or use the other-modes profile |
| The classifier blocks a Director `cafleet` command | Auto mode judged the action outside the request | Review the actual refusal under operator authority and preserve native/managed denies; operator-managed allow rules follow [`permissions.allow` coverage](cli-options.md#permissionsallow-coverage). |
| Auto is unavailable or an approval remains unresolved | Host eligibility, explicit ask rules or another native approval boundary applies | Surface the actual mode or pending request to the Director; inspect eligibility and settings under the host's existing authority before continuing. |

Verify native Monitor authorization separately from Bash broker rules. Monitor events remain broker events during an open user decision. Actual expiry, cancellation, child cleanup and independently observed continuation preservation require acceptance evidence.

## Codex {#codex}

The objective belongs to the thread, with model-callable lifecycle tools and optional token-budget accounting. Retain one native execution session handle for Director inbox reception and explicitly read it while delegated work remains, independently of whether a host goal exists. The installed CAFleet skill's Codex native instructions own tool calls, bounded reads, leases and recovery; the [manual lifecycle](../how-to/mixed-backend-team.md#manual-lifecycle) explains operation. Preserve supplied goal activation, pause, completion and budget authority. A background process alone supplies no unsolicited per-line wake after a model turn ends; fully idle Codex reconciles its inbox on the next user/native activation.

A native shell probe displayed the actual command, exit code, duration and output. Installed protocol metadata describes unsandboxed dispatch while public CLI guidance describes current approval/sandbox settings; the TUI's effective boundary and protocol path were not established. Treat native input as a Director-authorized action rather than assume a second member-tool permission check.

`--sandbox workspace-write` confines writes to the workspace under a
kernel-enforced sandbox — codex is the only backend with one.
`--approve-for-me` selects automatic approval review while the explicit
sandbox flag retains workspace protection. Native rules and managed policy
remain authoritative. An unavailable reviewer, unresolved approval or returned
refusal is surfaced to the Director with actual evidence; ordinary command
errors follow normal recovery. See upstream
[auto-review](https://learn.chatgpt.com/docs/sandboxing/auto-review).
Selecting these arguments establishes launch intent, not a live review result.

The `~/.codex/config.toml` prerequisites must be in place before the first
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

The preset's `shell` ruleset has a `"*": "deny"` base followed by its literal
allow entries and explicit `cafleet member prompt *` / `cafleet member exec *`
denies. OpenCode selects the **last** matching rule, so those dispatch denies
override its broader `cafleet *` allow. The preset is the source of truth for
routine-command patterns; an unmatched shell command is denied without a
permission prompt. Broad allow patterns grant no Director-only actions or Git
write authority to a member. The shared runtime instruction routes an observed
permission denial with command/cwd/environment/refusal evidence, including
for ad-hoc assignments that load no skill. Native read/edit boundaries and every
other tool permission remain as declared in the preset. The assigned role and
Director instructions determine which broker identities and file operations
a member may use. Capture and relay unresolved permission prompts to the Director.

The selected preset applies its fixed allow/deny policy in the existing mini
launch. Members execute permitted role-authorized commands and route observed
refusals with actual evidence. Surface unresolved approval to the Director;
supervision handles prompts that prevent a member from sending. Preserve
native deny boundaries. The declaration/ordering and argv tests establish
packaged behavior, rather than live permission-engine acceptance.

### Safety-floor caveats {#safety-floor-caveats}

The shell posture permits the preset's declared patterns over a deny base,
with no OS-level sandbox. Unmatched shell-tool commands fall to that base. Apply these
limits when choosing tools and authorizing commands:

| Surface | Example | Usage limit |
|---|---|---|
| MCP-contributed tools | — | Their participation in preset permission evaluation is unverified on opencode 2.1.4. Keep loaded configurations free of MCP servers. |
| Compound shell commands | <code>git log --stat; curl … &#124; sh</code> | Permission matching for compound lines is unverified on opencode 2.1.4. Submit one command per invocation and authorize each actual command. |
| Director-authorized native input | A task command can execute workspace code or hooks | Authorize the exact native command and the code/hooks it can execute under the host's existing permissions. |

cafleet ships no MCP stanzas. Operators MUST keep every loaded opencode
configuration free of MCP servers.

For kernel-enforced isolation, use the `codex` backend.

Spawn, preset loading, broker input and native shell input are validated against
opencode 2.1.4; opencode 2 is the minimum supported version.
