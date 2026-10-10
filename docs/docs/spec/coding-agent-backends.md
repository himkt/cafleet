# Coding-agent backends

Every member pane runs one of three coding-agent binaries: **claude** (Claude
Code), **codex** (OpenAI Codex CLI), or **opencode**. The backend is recorded
per member in `member_placements.coding_agent`; selection and inheritance via
`--coding-agent`, mixed-backend teams, and identity delivery are covered in
[Coding agents](../concepts/coding-agents.md). This page specifies each
backend's spawn argv, auto-approval posture, model-flag format, and
version/config requirements.

## Spawn argv {#spawn-argv}

| Backend | Spawn argv |
|---|---|
| `claude` | `claude --permission-mode dontAsk --allowedTools <rule>... --name <member-name> <prompt>` |
| `codex` | `codex --ask-for-approval never --sandbox workspace-write <prompt>` |
| `opencode` | `opencode --agent cafleet --prompt <prompt>` |

Shared contract:

- All three postures enable the Bash tool with no runtime permission prompts.
- All three honor the leading-`!` shell shortcut that
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
| `codex` | kernel-enforced ([Codex](#codex)) | no — locate the pane through `cafleet member list` (`pane_id` is ground truth) | Runs cafleet and any shell command directly | `~/.codex/rules/cafleet.rules` (`CODEX_HOME` relocates it), plus the `~/.codex/config.toml` settings and a trusted working directory — a permission posture, not a spawn dependency ([the rules file](#cafleet-rules-file)) |
| `opencode` | none | no — locate the pane through `cafleet member list` | Deny-by-default allowlist; everything outside it routes to the Director | `~/.opencode/agents/cafleet.md` (`OPENCODE_CONFIG_DIR` relocates it) — a spawn precondition ([the agent preset](#cafleet-agent-preset)) |

## Model selection {#model-selection}

| Backend | Accepted value format | Example values | Create-time validation |
|---|---|---|---|
| `claude` | Passes through verbatim | `haiku`, `sonnet`, `opus`, `fable` | none — the binary rejects unknown models |
| `codex` | Passes through verbatim | `gpt-5.6-sol` (default), `gpt-5.6-terra`, `gpt-5.6-luna`, `gpt-5.5` | none — the binary rejects unknown models |
| `opencode` | `<provider-id>/<model-id>`, split on the **first** `/` into two non-empty segments (model ids may contain further slashes) | `anthropic/claude-sonnet-4-6`, `openai/gpt-5.5` | exit 2 with `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').` |

## Reasoning effort {#reasoning-effort}

| Backend | Accepted levels | Forwarded as | Rejected with (exit 2) |
|---|---|---|---|
| `claude` | `low`, `medium`, `high`, `xhigh`, `max` | `--effort <level>`, immediately after the model tokens | `Error: --effort for the claude backend must be one of low, medium, high, xhigh, max (got '<value>').` |
| `codex` | `minimal`, `low`, `medium`, `high`, `xhigh` | the single token `--config=model_reasoning_effort=<level>`, immediately after the model tokens | `Error: --effort for the codex backend must be one of minimal, low, medium, high, xhigh (got '<value>').` |
| `opencode` | none — the backend exposes no reasoning-effort control | — | `Error: opencode does not support reasoning effort.` |

## Claude {#claude}

`--permission-mode dontAsk` is the reference auto-approval posture the other
backends match. A member runs four kinds of call without a prompt: its
[spawn-time allow rules](#spawn-time-allow-rules), Claude Code's built-in
read-only commands, file reads inside its working directory, and every tool
call matching `permissions.allow`. Every other call — a shell command or a
file edit, including one matching `permissions.ask` — is denied without a
prompt. A write to one of Claude Code's protected paths, such as a project's
`.git` or `.claude` directory, is denied whatever the allow list holds. Beyond
the broker commands, the operator's settings are therefore the member's allow
list; the recommended profiles are in
[Claude Code configuration](../quickstart.md#claude-code).

### Spawn-time allow rules {#spawn-time-allow-rules}

cafleet passes the broker commands a claude member needs as session allow
rules on its spawn command line, so the member does not depend on any
`Bash(cafleet ...)` rule in the user's `settings.json`:

| Member | Rules |
|---|---|
| Every claude member | `Bash(cafleet message *)` |
| The monitor member, additionally | `Bash(cafleet monitor scan *)`, `Bash(cafleet member ping *)` |

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
- A `deny` rule or hook added after the member is spawned is not seen by
  `cafleet doctor`, and the [ready watchdog](../concepts/monitoring.md#ready-watchdog)
  does not report a member that has already spoken. The runtime signal is the
  monitor member: it pings a member that stays quiet and reports one that is
  unchanged after the ping, using commands that are themselves spawn-allowed.
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

## Codex {#codex}

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
    pattern = ["cafleet", "monitor", "scan"],
    decision = "allow",
    match = ["cafleet monitor scan 1", "cafleet monitor scan 1 --lines 120 --json"],
    not_match = ["cafleet monitor 1", "cafleet message poll 5"],
)

prefix_rule(
    pattern = ["cafleet", "member", "ping"],
    decision = "allow",
    match = ["cafleet member ping 5", "cafleet member ping 5 --json"],
    not_match = ["cafleet member prompt 5 hello", "cafleet member list 1"],
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

The pane runs the bare `opencode` TUI (not `opencode run`), so it stays a
long-lived, observable pane like the other backends. The prompt is passed via
`--prompt` — bare `opencode`'s positional is a project path, not a message.

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

The preset's `bash` ruleset is deny-by-default: a `"*": "deny"` base first,
then an explicit allowlist translated from the operator's Claude Code
`permissions.allow` set (`cafleet *`, non-destructive `git` subcommands,
file-inspection utilities, and the project's cargo-backed mise tasks). opencode selects the
**last** matching rule, so this order is the safety floor — every check
resolves to `allow` or `deny`, never `ask`. A `cafleet member exec *` deny
entry follows the `cafleet *` allow, so it wins: the preset configures
members, and dispatching a command into another pane is the Director's act. A permission popup in an opencode pane is
therefore a regression escape, not a runtime decision: capture the pane,
escalate, and extend the allowlist by operator decision — do not answer the
popup ad hoc.

### Safety-floor caveats {#safety-floor-caveats}

The posture is a deny-by-default allowlist, with no OS-level sandbox.
Standalone un-enumerated commands fall to the `"*": "deny"` base, but three
classes of bypass under allowed globs persist:

| Bypass class | Example | Why the allowlist misses it |
|---|---|---|
| MCP-contributed tools | — | They bypass the permission evaluator entirely |
| Shell chaining or argument-space abuse inside an allowed `cmd *` match | <code>git log --stat; curl … &#124; sh</code> matching `git log *` | The compound line's leading tokens match the allowed glob |
| Interpreter or hook execution via allowed tooling | `mise //cafleet:test *` executes workspace-writable test code; `git commit *` runs `.git/hooks` | The allowed command is itself the execution vector |

cafleet ships no MCP stanzas, and operators MUST NOT add MCP servers to any
opencode config their machine loads.

For kernel-enforced isolation, use the `codex` backend.

Validated against `opencode 1.15.5` (the minimum supported version).
