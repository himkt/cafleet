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
      "Bash(cafleet * pane exec *)"
    ]
  }
}
```

The `Bash(cafleet *)` pattern is the single allow-everything entry that the
literal `--fleet-id <int>` / `--agent-id <int>` flag convention enables —
one pattern covers every subcommand for every fleet. `cafleet pane exec *`
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
prefix_rule(pattern = ["cafleet", "agent"],      decision = "allow")
prefix_rule(pattern = ["cafleet", "message"],    decision = "allow")
prefix_rule(pattern = ["cafleet", "monitor"],    decision = "allow")
prefix_rule(pattern = ["cafleet", "pane", "capture"], decision = "allow")
prefix_rule(pattern = ["cafleet", "pane", "input"],   decision = "allow")
prefix_rule(pattern = ["cafleet", "pane", "wake"],    decision = "allow")

prefix_rule(
    pattern = ["cafleet", "pane", "exec"],
    decision = "prompt",
    justification = "cafleet pane exec runs arbitrary commands on a member",
)
```

Unlike Claude Code's `Bash(cafleet *)` glob — where `*` matches any token
sequence — Codex's `prefix_rule` is a positional prefix matcher, so a broad
`["cafleet"]` allow would also cover `cafleet pane exec`. Enumerating the safe
subgroups (and the safe `pane` subcommands) explicitly keeps the `pane exec`
prompt rule effective.

Because `--fleet-id` is a per-subcommand option — a trailing argument after the
subcommand name (e.g. `cafleet message send --fleet-id <int> ...`) — the
positional `prefix_rule`s above already cover every fleet-scoped invocation: the
fleet id sits past the matched prefix, so no per-fleet rule is needed.
`cafleet pane exec --fleet-id <int> ...` still matches the
`["cafleet", "pane", "exec"]` prompt rule (the prefix is matched before the
trailing `--fleet-id`), so `pane exec` keeps prompting.

## Opencode

!!! tip "Where this lives"

    Opencode's `cafleet` agent definition lives at `~/.opencode/agents/cafleet.md`.

No manual configuration is required. On the first `cafleet agent spawn
--coding-agent opencode` call, cafleet writes the `cafleet` agent definition
to `~/.opencode/agents/cafleet.md` if it does not already exist — the preset
embeds the catch-all-allow + specific-deny ruleset that mirrors Claude Code's
`dontAsk` safety floor. To refresh the preset after a CAFleet release (e.g.
after `pip install -U cafleet`), delete the existing file and re-run
`cafleet agent spawn --coding-agent opencode` so the next spawn writes the
current bundled preset. See
[Opencode members](../reference/coding-agents/opencode.md) for the full
materialization protocol, refresh recipe, and the operator MUST-NOT rule on
MCP servers (MCP-contributed tools bypass the deny-list).

## Defaulting the fleet id with `CAFLEET_FLEET_ID`

Every fleet-scoped command takes a required `--fleet-id`. To avoid retyping it,
export `CAFLEET_FLEET_ID` — when set, it supplies the **default** for
`--fleet-id` on every command that takes it:

```bash
export CAFLEET_FLEET_ID=1
cafleet message poll --agent-id 4      # --fleet-id defaults to 1
cafleet message poll --fleet-id 2 --agent-id 4   # explicit flag overrides
```

An explicit `--fleet-id` always overrides the env default, and a non-integer
`CAFLEET_FLEET_ID` fails at parse time (exit 2). A spawned member already
receives `CAFLEET_FLEET_ID` in its pane environment, so its fleet is
unambiguous. Note that agents driving cafleet under `permissions.allow` still
pass `--fleet-id` as a literal flag — the allow patterns match the literal
command string, so a shell-expanded `$CAFLEET_FLEET_ID` would break the match
and prompt. The env default is a convenience for interactive shells; the literal
flag remains the canonical form in allow-listed agent invocations.

## Building docs locally

Once the CLI is installed, you can build the
documentation site (this site) locally with:

```bash
mise //:docs-build
```

That task is a thin wrapper around `uv run zensical build --clean` and is the
same command the GitHub Actions workflow runs.
