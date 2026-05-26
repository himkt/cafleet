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
      "Skill(cafleet:cafleet-agent-team-monitoring)",
      "Skill(cafleet:cafleet-agent-team-supervision)",
      "Skill(cafleet:cafleet-base-dir)",
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
literal `--session-id <uuid>` / `--agent-id <uuid>` flag convention enables —
one pattern covers every subcommand for every session. `cafleet member exec *`
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
prefix_rule(
    pattern = ["cafleet"],
    decision = "allow",
    justification = "All cafleet subcommands are allowed by default",
)

prefix_rule(
    pattern = ["cafleet", "member", "exec"],
    decision = "prompt",
    justification = "cafleet member exec runs arbitrary commands on a member",
)
```

This mirrors the Claude Code split: everything `cafleet *` is auto-allowed
except `cafleet member exec`, which is gated on a per-call prompt.

## Building docs locally

Once the CLI is installed and the plugin enabled, you can build the
documentation site (this site) locally with:

```bash
mise //:docs-build
```

That task is a thin wrapper around `uv run zensical build --clean` and is the
same command the GitHub Actions workflow runs. See
[Authoring](authoring.md) for the Markdown features you can use when writing
new pages.
