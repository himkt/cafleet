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
| `claude` | `claude --permission-mode dontAsk --name <member-name> <prompt>` |
| `codex` | `codex --ask-for-approval never --sandbox workspace-write <prompt>` |
| `opencode` | `opencode --agent cafleet --prompt <prompt>` |

Shared contract:

- All three postures enable the Bash tool with no runtime permission prompts.
- All three honor the leading-`!` shell shortcut that
  [`cafleet member prompt --shell`](cli-options.md#member-prompt) uses.
- `--model <m>` from `cafleet member create` is inserted immediately before
  the prompt. The value passes through verbatim — the binary rejects unknown
  models, so newly released models need no cafleet release. Omitted, no model
  tokens are emitted and the binary uses its configured default. Per-backend
  formats and create-time validation are in
  [Model selection](#model-selection).
- `--effort <level>` from `cafleet member create` forwards a reasoning-effort
  level, emitted immediately after the model tokens (before the prompt).
  Unlike `--model`, the accepted level set is validated per backend at create
  time — an unknown level exits 2 before any registration or multiplexer side
  effect. Omitted, no effort tokens are emitted and the argv is byte-identical
  to the no-effort form. Per-backend levels and rejection strings are in
  [Reasoning effort](#reasoning-effort).
- A missing binary fails the spawn: exit 1 with
  `Error: binary <name> not found on PATH`.

Per-backend capabilities:

| Backend | OS-level sandbox | Sets the pane title | Shell-command posture | Preset / config prerequisite |
|---|---|---|---|---|
| `claude` | none | yes, via `--name <member-name>` | Runs cafleet and any shell command directly | none |
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
backends match.

## Codex {#codex}

`--sandbox workspace-write` confines writes to the workspace under a
kernel-enforced sandbox — codex is the only backend with one.
`--ask-for-approval never` disables interactive approval prompts (upstream
write-up: <https://developers.openai.com/codex/agent-approvals-security>).

Three `~/.codex/config.toml` prerequisites must be in place before the first
codex spawn, covered in
[Codex configuration](../concepts/coding-agents.md#codex) and
[Trust the working directory](../concepts/coding-agents.md#trust-the-working-directory):

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
a static file in the assets release archive (`presets/codex/cafleet.rules`)
and is installed by `cafleet setup`:

```text
prefix_rule(pattern = ["cafleet"], decision = "allow")

prefix_rule(
    pattern = ["cafleet", "member", "prompt"],
    decision = "prompt",
    justification = "cafleet member prompt keystrokes arbitrary text or shell commands into a member pane",
)
```

Codex applies the strictest decision when more than one rule matches
(`forbidden` > `prompt` > `allow`): `cafleet member prompt` matches both rules,
so its `prompt` wins and each invocation keeps requiring approval, while every
other subcommand matches only the broad `["cafleet"]` allow — for every fleet,
since every id rides past the matched prefix as a positional or trailing
argument.

The file is **owned by `cafleet setup`**: it is overwritten on every install,
so operator customizations belong in a separate rules file under
`~/.codex/rules/` — Codex loads every `*.rules` file in that directory at
startup and applies the strictest decision across all of them. The rules file
is a permission posture, not a spawn dependency: `cafleet member create
--coding-agent codex` requires only the `codex` binary on PATH.

## Opencode {#opencode}

The pane runs the bare `opencode` TUI (not `opencode run`), so it stays a
long-lived, observable pane like the other backends. The prompt is passed via
`--prompt` — bare `opencode`'s positional is a project path, not a message.

### The `cafleet` agent preset {#cafleet-agent-preset}

`--agent cafleet` binds the member to `~/.opencode/agents/cafleet.md`
(`OPENCODE_CONFIG_DIR` relocates the `~/.opencode` base — see
[Config-dir resolution](cli-options.md#config-dir-resolution); the spawn
precondition checks the same resolved path `setup` installs to). The
preset ships as a static file in the assets release archive
(`presets/opencode/cafleet.md`) and is installed — overwriting any existing
copy — by `cafleet setup`; to refresh after a CAFleet upgrade, re-run it. The
preset is a spawn precondition: the spawn argv references `--agent cafleet`,
so `cafleet member create --coding-agent opencode` fails with `opencode agent
preset not found at <preset>; run 'cafleet setup --coding-agent opencode'
first` when the file is missing.

The preset's `bash` ruleset is deny-by-default: a `"*": "deny"` base first,
then an explicit allowlist translated from the operator's Claude Code
`permissions.allow` set (`cafleet *`, non-destructive `git` subcommands,
file-inspection utilities, and the project's cargo-backed mise tasks). opencode selects the
**last** matching rule, so this order is the safety floor — every check
resolves to `allow` or `deny`, never `ask`. A permission popup in an opencode pane is
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
