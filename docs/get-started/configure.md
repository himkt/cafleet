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
prefix_rule(pattern = ["cafleet", "--version"],  decision = "allow")
prefix_rule(pattern = ["cafleet", "doctor"],     decision = "allow")
prefix_rule(pattern = ["cafleet", "server"],     decision = "allow")
prefix_rule(pattern = ["cafleet", "db"],         decision = "allow")
prefix_rule(pattern = ["cafleet", "session"],    decision = "allow")
prefix_rule(pattern = ["cafleet", "base-dir"],   decision = "allow")
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

The `cafleet --session-id <uuid> <subgroup> …` invocations also need to be
allowed; since `--session-id` and its UUID sit between `cafleet` and the
subgroup name, add a per-session allow for the specific UUIDs you use:

```text
prefix_rule(pattern = ["cafleet", "--session-id", "<your-session-uuid>"], decision = "allow")
prefix_rule(pattern = ["cafleet", "--session-id", "<your-session-uuid>", "member", "exec"],
            decision = "prompt",
            justification = "cafleet member exec runs arbitrary commands on a member")
```

Repeat the pair for every session UUID you operate. The per-session prompt
rule is more specific than the per-session allow, so `member exec` keeps
prompting even with the broader allow in place.

## Building docs locally

Once the CLI is installed and the plugin enabled, you can build the
documentation site (this site) locally with:

```bash
mise //:docs-build
```

That task is a thin wrapper around `uv run zensical build --clean` and is the
same command the GitHub Actions workflow runs.
