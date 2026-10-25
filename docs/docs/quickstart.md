# Quickstart

Run CAFleet from a trusted coding-agent workspace inside tmux or herdr. Install the selected agent on PATH, then create a fleet and delegate work.

## Install

```bash
brew install himkt/tap/cafleet
```

```bash
cafleet setup
```

Alternatively, extract the single binary for your platform from [GitHub Releases](https://github.com/himkt/cafleet/releases) onto PATH and run setup. Installation works offline from the binary's embedded skills/presets. Run setup after upgrading; use `--coding-agent` to select a backend.

Setup first validates/migrates compatible storage, then installs selected backend assets sequentially. A failure stops remaining work; completed installs retain their success records. Existing supported databases upgrade in place. An ahead/foreign/malformed database is refused before assets or data change; preserve that file and choose the appropriate CLI or an empty database path.

## Configure

[Backend configuration](spec/coding-agent-backends.md) explains host permissions and launch behavior. The [CLI reference](spec/cli-options.md#config-dir-resolution) owns configuration-path rules.

### Claude Code

Members use `dontAsk` and receive a broker allow rule at spawn. Add task-specific permissions and skill/file access to your user settings. For a Director using auto mode, this starting profile leaves command classification active:

```json
{
  "permissions": {
    "defaultMode": "auto",
    "allow": [
      "Skill(cafleet)",
      "Skill(cafleet-design-doc)",
      "Read(~/.claude/skills/**)",
      "Edit",
      "Write"
    ],
    "ask": [
      "Bash(cafleet member prompt *)",
      "Bash(cafleet member exec *)"
    ]
  }
}
```

Set auto mode in user/managed settings. For another Director permission mode, omit `defaultMode` and add `Bash(cafleet *)` to `allow`. Keep native pane actions under `ask`; they submit arbitrary composer text or shell commands.

`Edit`/`Write` authorize file editing for every session using these settings. Confine them with a suitable workspace path rule when needed. Protected paths require Director editing or matching host authorization. When relocating configuration, adjust the skill read path accordingly.

Add allow rules for the project's actual task commands so members can perform their assigned checks. A deny/ask rule still governs matching member commands. Doctor reports environment facts; actual host enforcement remains authoritative.

### Codex

Setup installs the CAFleet rules file. Configure the database directory and local multiplexer socket access in `~/.codex/config.toml`:

```toml
[sandbox_workspace_write]
network_access = true
writable_roots = ["/home/<you>/.local/share/cafleet"]
```

Use the absolute parent of your selected database. The multiplexer socket needs network permission under the sandbox. Members use `--ask-for-approval never --sandbox workspace-write`; host sandbox/rules remain authoritative. Keep custom rules separate from the setup-owned `cafleet.rules`, which setup replaces.

### OpenCode

Setup installs the `cafleet` agent preset. Use OpenCode 2 or newer. Its declared tool permissions apply to spawned members; required denied commands route to the Director. Keep loaded configuration free of MCP servers while their preset enforcement is unverified. The preset's permissions and the assigned role jointly determine allowed work.

### Trust the working directory

Launch the selected coding agent once in the intended workspace and accept its trust prompt before spawning members. Each worktree needs its own trust. A first-run trust prompt can block a new member before it processes messages.

For Codex, the trust entry uses the absolute workspace:

```toml
[projects."/absolute/workspace"]
trust_level = "trusted"
```

## Start a team

Ask your agent:

```text
Use CAFleet to delegate independent reviews of this change, collect the reports, resolve findings and close the team.
```

The Director supplies scoped assignments, receives ready, reads complete durable replies and owns team lifecycle. Follow [Run a fleet](how-to/mixed-backend-team.md) for explicit commands and recovery.

## Diagnose

```bash
cafleet doctor --json
```

Doctor collects independent multiplexer, database and backend checks. Resolve error checks; optional missing backends and stale/missing install records are informational. Setup repairs a missing/behind database and refreshes embedded assets. Recorded install versions are diagnostic facts; existing ready fleets can continue durable messaging.
