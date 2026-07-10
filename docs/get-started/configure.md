---
icon: lucide/settings
---

# Configure

CAFleet is designed to run inside a coding agent without per-command permission
prompts. Each backend has a different config file and permission system; the
snippets below are the recommended starting points.

## Claude Code

!!! tip "Where this lives"

    Typically the config entries below go in `~/.claude/settings.json`.

```json
{
  "permissions": {
    "allow": [
      "Bash(cafleet *)",
      "Skill(cafleet:cafleet)",
      "Skill(cafleet:cafleet-design-doc)",
      "Skill(cafleet:cafleet-research)"
    ],
    "ask": [
      "Bash(cafleet * member exec *)"
    ]
  }
}
```

The `Bash(cafleet *)` pattern is the single allow-everything entry that the
literal `--fleet-id <int>` / `--agent-id <int>` flag convention enables —
one pattern covers every subcommand for every fleet. `cafleet member exec *`
is moved to the `ask` list because it dispatches arbitrary shell commands on
behalf of a member; the operator should confirm each invocation.

## Codex

!!! tip "Where this lives"

    Typically the config entries below go in `~/.codex/config.toml`.

```toml
[sandbox_workspace_write]
writable_roots = ["/home/<you>/.local/share/cafleet"]
```

The recommended Codex rules for `cafleet` commands live at
`~/.codex/rules/cafleet.rules`:

```text
prefix_rule(pattern = ["cafleet", "--version"],  decision = "allow")
prefix_rule(pattern = ["cafleet", "setup"],      decision = "allow")
prefix_rule(pattern = ["cafleet", "doctor"],     decision = "allow")
prefix_rule(pattern = ["cafleet", "server"],     decision = "allow")
prefix_rule(pattern = ["cafleet", "fleet"],      decision = "allow")
prefix_rule(pattern = ["cafleet", "message"],    decision = "allow")
prefix_rule(pattern = ["cafleet", "monitor"],    decision = "allow")
prefix_rule(pattern = ["cafleet", "member", "create"],  decision = "allow")
prefix_rule(pattern = ["cafleet", "member", "delete"],  decision = "allow")
prefix_rule(pattern = ["cafleet", "member", "show"],    decision = "allow")
prefix_rule(pattern = ["cafleet", "member", "list"],    decision = "allow")
prefix_rule(pattern = ["cafleet", "member", "capture"], decision = "allow")
prefix_rule(pattern = ["cafleet", "member", "ping"],    decision = "allow")
prefix_rule(pattern = ["cafleet", "member", "nudge"],   decision = "allow")

prefix_rule(
    pattern = ["cafleet", "member", "exec"],
    decision = "prompt",
    justification = "cafleet member exec runs arbitrary commands on a member",
)
```

Unlike Claude Code's `Bash(cafleet *)` glob — where `*` matches any token
sequence — Codex's `prefix_rule` is a positional prefix matcher, so a broad
`["cafleet"]` allow would also cover `cafleet member exec`. Enumerating the safe
subgroups (and the safe `member` subcommands) explicitly keeps the `member exec`
prompt rule effective.

Because `--fleet-id` is a per-subcommand option — a trailing argument after the
subcommand name (e.g. `cafleet message send --fleet-id <int> ...`) — the
positional `prefix_rule`s above already cover every fleet-scoped invocation: the
fleet id sits past the matched prefix, so no per-fleet rule is needed.
`cafleet member exec --fleet-id <int> ...` still matches the
`["cafleet", "member", "exec"]` prompt rule (the prefix is matched before the
trailing `--fleet-id`), so `member exec` keeps prompting.

## Opencode

!!! tip "Where this lives"

    Opencode's `cafleet` agent definition lives at `~/.opencode/agents/cafleet.md`.

No manual configuration is required. On the first `cafleet member create
--coding-agent opencode` call, cafleet writes the `cafleet` agent definition
to `~/.opencode/agents/cafleet.md` if it does not already exist — the preset
embeds the catch-all-allow + specific-deny ruleset that mirrors Claude Code's
`dontAsk` safety floor. To refresh the preset after a CAFleet release (e.g.
after `pip install -U cafleet`), delete the existing file and re-run
`cafleet member create --coding-agent opencode` so the next spawn writes the
current bundled preset. See
[Opencode members](../reference/coding-agents/opencode.md) for the full
materialization protocol, refresh recipe, and the operator MUST-NOT rule on
MCP servers (MCP-contributed tools bypass the deny-list).

## Trust the working directory

Coding agents ask for a trust confirmation the first time they start in a
directory they have not seen before. Trust the workspace in advance: launch
your coding agent once in the working directory the member panes will run in
and accept its first-run prompt, or add a trust entry to the agent's
configuration file (see your agent's reference page). Trust is granted per
directory, so each git worktree needs its own approval.

This prevents a spawn-time stall: in an untrusted directory, the agent's
first-run trust prompt stalls a freshly spawned member — the member ignores
every incoming message until the prompt is cleared.

## Passing the fleet id

Every fleet-scoped command except `fleet create` and `fleet list` takes a
required `--fleet-id`, passed as a literal integer flag on each invocation (a
member reads its fleet id from the `FLEET ID:` line of its spawn prompt). Agents driving cafleet under
`permissions.allow` pass `--fleet-id` as a literal flag — the allow patterns
match the literal command string, so a shell-expanded variable would break the
match and prompt.
