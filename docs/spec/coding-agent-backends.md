---
icon: lucide/bot
---

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
  claude and codex members run cafleet (and any shell command) directly;
  opencode members run cafleet and the other commands on the preset's
  deny-by-default allowlist, and route everything else to the Director.
- All three honor the leading-`!` shell shortcut that
  [`cafleet member exec`](cli-options.md#member-exec) uses.
- `--model <m>` from `cafleet member create` is inserted immediately before
  the prompt. The value passes through verbatim — the binary rejects unknown
  models, so newly released models need no cafleet release. Omitted, no model
  tokens are emitted and the binary uses its configured default. (The opencode
  backend additionally validates the value's format — see below.)
- `--effort <level>` from `cafleet member create` forwards a reasoning-effort
  level, emitted immediately after the model tokens (before the prompt).
  Unlike `--model`, the accepted level set is validated per backend at create
  time — an unknown level exits 2 before any registration or multiplexer side
  effect. Omitted, no effort tokens are emitted and the argv is byte-identical
  to the no-effort form. opencode does not support the flag — see below.
- A missing binary fails the spawn: exit 1 with
  `Error: binary <name> not found on PATH`.
- Only `claude` sets the pane title (via `--name`); locate `codex` and
  `opencode` panes through `cafleet member list` (`pane_id` is ground truth).

## Claude {#claude}

`--permission-mode dontAsk` is the reference auto-approval posture the other
backends match. Example `--model` values: `haiku`, `sonnet`, `opus`, `fable`.

Reasoning-effort levels: `low`, `medium`, `high`, `xhigh`, `max`, forwarded
as `--effort <level>` immediately after the model tokens. An unknown level
fails at create time with exit 2 and
`Error: --effort for the claude backend must be one of low, medium, high, xhigh, max (got '<value>').`

## Codex {#codex}

`--sandbox workspace-write` confines writes to the workspace under a
kernel-enforced sandbox — codex is the only backend with one.
`--ask-for-approval never` disables interactive approval prompts (upstream
write-up: <https://developers.openai.com/codex/agent-approvals-security>).
Example `--model` values: `gpt-5.6-sol` (default), `gpt-5.6-terra`,
`gpt-5.6-luna`, `gpt-5.5`, `gpt-5.4`, `gpt-5.4-mini`.

Reasoning-effort levels: `minimal`, `low`, `medium`, `high`, `xhigh`,
forwarded as the single token `--config=model_reasoning_effort=<level>`
immediately after the model tokens. An unknown level fails at create time
with exit 2 and
`Error: --effort for the codex backend must be one of minimal, low, medium, high, xhigh (got '<value>').`

Two `~/.codex/config.toml` requirements, covered in
[Quickstart § Configure](../quickstart.md#codex): `network_access = true`
(the multiplexer socket counts as network access) and `writable_roots`
including the cafleet DB directory. The working directory must also be
trusted before spawning
([Quickstart § Trust the working directory](../quickstart.md#trust-the-working-directory)):

```toml
[projects."/abs/path/to/workspace"]
trust_level = "trusted"
```

### The `cafleet` rules file {#cafleet-rules-file}

`~/.codex/rules/cafleet.rules` grants the auto-approval posture for `cafleet`
commands. It ships as a static file in the assets release archive
(`presets/codex/cafleet.rules`) and is installed by `cafleet setup`:

```text
prefix_rule(pattern = ["cafleet"], decision = "allow")

prefix_rule(
    pattern = ["cafleet", "member", "exec"],
    decision = "prompt",
    justification = "cafleet member exec runs arbitrary commands on a member",
)
```

Codex applies the strictest decision when more than one rule matches
(`forbidden` > `prompt` > `allow`): `cafleet member exec` matches both rules,
so its `prompt` wins and each invocation keeps requiring approval, while every
other subcommand matches only the broad `["cafleet"]` allow — for every fleet,
since `--fleet-id` is a trailing flag past the matched prefix.

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

`--model` values must be `<provider-id>/<model-id>`, split on the **first**
`/` into two non-empty segments (model ids may contain further slashes).
Violations fail at create time with exit 2 and
`Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').`
Example values: `anthropic/claude-sonnet-4-6`, `openai/gpt-5.5`.

opencode exposes no reasoning-effort control: `--effort` with this backend
fails at create time with exit 2 and
`Error: opencode does not support reasoning effort.`, before any side effect.

### The `cafleet` agent preset {#cafleet-agent-preset}

`--agent cafleet` binds the member to `~/.opencode/agents/cafleet.md`. The
preset ships as a static file in the assets release archive
(`presets/opencode/cafleet.md`) and is installed — overwriting any existing
copy — by `cafleet setup`; to refresh after a CAFleet upgrade, re-run it. The
preset is a spawn precondition: the spawn argv references `--agent cafleet`,
so `cafleet member create --coding-agent opencode` fails with `opencode agent
preset not found at <preset>; run 'cafleet setup' first` when the file is
missing.

The preset's `bash` ruleset is deny-by-default: a `"*": "deny"` base first,
then an explicit allowlist translated from the operator's Claude Code
`permissions.allow` set (`cafleet *`, non-destructive `git` subcommands,
file-inspection utilities, and Python project tooling). opencode selects the
**last** matching rule, so this order is the safety floor — every check
resolves to `allow` or `deny`, never `ask`. A permission popup in an opencode pane is
therefore a regression escape, not a runtime decision: capture the pane,
escalate, and extend the allowlist by operator decision — do not answer the
popup ad hoc.

### Safety-floor caveats {#safety-floor-caveats}

The posture is a deny-by-default allowlist, with no OS-level sandbox:

- **MCP-contributed tools bypass the permission evaluator.** cafleet ships no
  MCP stanzas; operators MUST NOT add MCP servers to any opencode config
  their machine loads.
- Standalone un-enumerated commands fall to the `"*": "deny"` base, but
  bypasses under allowed globs persist: argument-space abuse and shell
  chaining inside a `cmd *` match (a compound line whose leading tokens match
  an allowed glob, e.g. `git log --stat; curl … | sh` matching `git log *`),
  and interpreter/hook execution via allowed tooling (`uv run pytest *`
  executes workspace-writable test code; `git commit *` runs `.git/hooks`) —
  unless opencode's per-sub-command evaluation of compound lines is verified
  and recorded here. For kernel-enforced isolation, use the `codex` backend.

Validated against `opencode 1.15.5` (the minimum supported version).
