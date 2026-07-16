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

- All three postures enable the Bash tool and auto-resolve routine permission
  prompts, so members run cafleet (and any shell command) directly.
- All three honor the leading-`!` shell shortcut that
  [`cafleet member exec`](cli-options.md#member-exec) uses.
- `--model <m>` from `cafleet member create` is inserted immediately before
  the prompt. The value passes through verbatim — the binary rejects unknown
  models, so newly released models need no cafleet release. Omitted, no model
  tokens are emitted and the binary uses its configured default. (The opencode
  backend additionally validates the value's format — see below.)
- A missing binary fails the spawn: exit 1 with
  `Error: binary <name> not found on PATH`.
- Only `claude` sets the pane title (via `--name`); locate `codex` and
  `opencode` panes through `cafleet member list` (`pane_id` is ground truth).

## Claude {#claude}

`--permission-mode dontAsk` is the reference auto-approval posture the other
backends match. Example `--model` values: `fable`, `opus`, `sonnet`, `haiku`,
`best`, `default`, `opusplan`, `sonnet[1m]`, `opus[1m]`.

## Codex {#codex}

`--sandbox workspace-write` confines writes to the workspace under a
kernel-enforced sandbox — codex is the only backend with one.
`--ask-for-approval never` disables interactive approval prompts (upstream
write-up: <https://developers.openai.com/codex/agent-approvals-security>).
Example `--model` values: `gpt-5.5`, `gpt-5.4`, `gpt-5.4-mini`.

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

Validated against `codex-cli 0.128.0`; older versions may reject the
approval/sandbox flags.

## Opencode {#opencode}

The pane runs the bare `opencode` TUI (not `opencode run`), so it stays a
long-lived, observable pane like the other backends. The prompt is passed via
`--prompt` — bare `opencode`'s positional is a project path, not a message.

`--model` values must be `<provider-id>/<model-id>`, split on the **first**
`/` into two non-empty segments (model ids may contain further slashes).
Violations fail at create time with exit 2 and
`Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').`
Example values: `anthropic/claude-sonnet-4-6`, `openai/gpt-5.5`.

### The `cafleet` agent preset {#cafleet-agent-preset}

`--agent cafleet` binds the member to `~/.opencode/agents/cafleet.md` — the
only file cafleet ever writes under `$HOME`. It is materialized on first
spawn with skip-if-exists semantics: operator customizations are never
overwritten. To refresh after a CAFleet upgrade, delete the file and spawn an
opencode member; the next spawn re-renders the current bundled preset.

The preset's ruleset lists a catch-all `"*": "allow"` first, then specific
denies (`sudo*`, `rm -rf*`, `curl*`, `git push*`, `**/.env`, …). opencode
selects the **last** matching rule, so this order is the safety floor —
every check resolves to `allow` or `deny`, never `ask`. A permission popup
in an opencode pane is therefore a regression escape, not a runtime decision:
capture the pane, escalate, and extend the deny-list — do not answer the
popup ad hoc.

### Safety-floor caveats {#safety-floor-caveats}

The posture is deny-list only, with no OS-level sandbox:

- **MCP-contributed tools bypass the permission evaluator.** cafleet ships no
  MCP stanzas; operators MUST NOT add MCP servers to any opencode config
  their machine loads.
- Un-enumerated shell wrappers, in-language eval, and side-channel egress
  also bypass the deny-list. For kernel-enforced isolation, use the `codex`
  backend.

Validated against `opencode 1.15.5` (the minimum supported version).
