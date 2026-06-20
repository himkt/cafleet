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
      "Skill(cafleet:cafleet-create-figure)",
      "Skill(cafleet:cafleet-design-doc)",
      "Skill(cafleet:cafleet-design-doc-create)",
      "Skill(cafleet:cafleet-design-doc-execute)",
      "Skill(cafleet:cafleet-design-doc-interview)",
      "Skill(cafleet:cafleet-my-slidev)",
      "Skill(cafleet:cafleet-research-presentation)",
      "Skill(cafleet:cafleet-research-report)"
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
[marketplaces.cafleet]
last_updated = "2026-05-16T11:56:51Z"
last_revision = "03a8caa66c8a7981345d74fe3aec9a6e498792a1"
source_type = "git"
source = "https://github.com/himkt/cafleet.git"

[plugins."cafleet@cafleet"]
enabled = true

[sandbox_workspace_write]
writable_roots = ["/home/<you>/.local/share/cafleet"]
```

The recommended Codex rules for `cafleet` commands live at
`~/.codex/rules/cafleet.rules`:

```text
prefix_rule(pattern = ["cafleet", "--version"],  decision = "allow")
prefix_rule(pattern = ["cafleet", "doctor"],     decision = "allow")
prefix_rule(pattern = ["cafleet", "server"],     decision = "allow")
prefix_rule(pattern = ["cafleet", "db"],         decision = "allow")
prefix_rule(pattern = ["cafleet", "fleet"],      decision = "allow")
prefix_rule(pattern = ["cafleet", "agent"],      decision = "allow")
prefix_rule(pattern = ["cafleet", "message"],    decision = "allow")
prefix_rule(pattern = ["cafleet", "member", "create"],     decision = "allow")
prefix_rule(pattern = ["cafleet", "member", "delete"],     decision = "allow")
prefix_rule(pattern = ["cafleet", "member", "list"],       decision = "allow")
prefix_rule(pattern = ["cafleet", "member", "capture"],    decision = "allow")
prefix_rule(pattern = ["cafleet", "member", "send-input"], decision = "allow")
prefix_rule(pattern = ["cafleet", "member", "ping"],       decision = "allow")

prefix_rule(
    pattern = ["cafleet", "member", "exec"],
    decision = "prompt",
    justification = "cafleet member exec runs arbitrary commands on a member",
)
```

Unlike Claude Code's `Bash(cafleet *)` glob — where `*` matches any token
sequence — Codex's `prefix_rule` is a positional prefix matcher, so a broad
`["cafleet"]` allow would also cover `cafleet member exec`. Enumerating the
safe subgroups explicitly keeps the `member exec` prompt rule effective.

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

## Building docs locally

Once the CLI is installed and the plugin enabled, you can build the
documentation site (this site) locally with:

```bash
mise //:docs-build
```

That task is a thin wrapper around `uv run zensical build --clean` and is the
same command the GitHub Actions workflow runs.
