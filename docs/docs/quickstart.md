# Quickstart

Create a fleet, send a message, and close the team from your coding-agent
pane inside tmux or herdr.

## Install

```bash
brew install himkt/tap/cafleet
```

```bash
cafleet setup
```

Alternatively, extract the binary for your platform from [GitHub Releases](https://github.com/himkt/cafleet/releases) onto PATH, then run setup. Setup installs embedded skills, presets and plugins offline; `--coding-agent` selects a backend.

Rerun `cafleet setup` after every upgrade, patch releases included: each version starts from a fresh database ([Upgrading cafleet](concepts/storage.md#upgrading-cafleet)).

## Configure

CAFleet runs inside a coding agent without per-command permission prompts.
Configure each backend you use:

| Backend | Config file | Manual configuration | Installed by `cafleet setup` |
|---|---|---|---|
| [`claude` (Claude Code)](#claude-code) | `~/.claude/settings.json` | The permission profile below that matches the Director's mode | The skills |
| [`codex` (OpenAI Codex CLI)](#codex) | `~/.codex/config.toml` | The `[sandbox_workspace_write]` entries below | The skills, plus `~/.codex/rules/cafleet.rules` |
| [`opencode`](#opencode) | — | — | The skills, the `cafleet` and `cafleet-ask` agent presets and the CAFleet plugin |
| [`hermes` (Hermes Agent)](#hermes) | `~/.hermes/config.yaml` | Keep `approvals.mode` at its `smart` default | The skills and the CAFleet plugin, enabled in your Hermes configuration |
| [`pi` (pi coding agent)](#pi) | — | — | The skills and the CAFleet pi extension |

The paths above are defaults ([Config directories](concepts/coding-agents.md#config-directories) covers relocating them); [Backend configuration](concepts/coding-agents.md#backend-configuration) has each backend's full reference and troubleshooting.

### Claude Code

Add the profile that matches your Director session's permission mode to
`~/.claude/settings.json`. Members always start in `auto` mode, so auto mode
must be available for your account and model.

#### Auto mode

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
      "Bash(cafleet member exec *)",
      "Bash(cafleet member press *)"
    ]
  }
}
```

#### Other permission modes

```json
{
  "permissions": {
    "allow": [
      "Bash(cafleet *)",
      "Skill(cafleet)",
      "Skill(cafleet-design-doc)",
      "Read(~/.claude/skills/**)",
      "Edit",
      "Write"
    ],
    "ask": [
      "Bash(cafleet member prompt *)",
      "Bash(cafleet member exec *)",
      "Bash(cafleet member press *)"
    ]
  }
}
```

The optional `ask` entries let you confirm each input the Director sends into
a member's pane; drop them to let it dispatch without a prompt. Add an allow
entry, such as `Bash(npm test *)`, for every project command members run.

### Codex

```toml
[sandbox_workspace_write]
network_access = true
writable_roots = ["/home/<you>/.local/share/cafleet"]
```

`network_access` lets cafleet reach the tmux or herdr socket, and `writable_roots` names the database directory. Setup overwrites the installed `~/.codex/rules/cafleet.rules`, so keep your own Codex rules in a separate file beside it.

Codex 0.167.0 and later run a Codex Director's tool commands through the shared Codex daemon, so registering a Codex Director usually asks you for its pane ID and server ([Director pane verification](concepts/members.md#director-pane-verification)).

### OpenCode

1. Launch the Director on OpenCode 2.2.0 or later with
   `opencode mini --standalone` from the project directory, with the default
   agent.
2. After registering the fleet, the Director starts reception with the
   plugin's `cafleet_listen` tool.
3. Manage a goal with `/cafleet-goal <objective>` and its `status`, `pause`,
   `resume` and `clear` subcommands.

Members run under the `cafleet-ask` preset: their broker message commands run
and every other shell command opens a native permission form for the Director.
Choose `--permission-mode cafleet` for a member whose other commands should be
denied and routed to the Director instead.

### Hermes

:::warning Start the Director with `hermes chat --tui`
In the classic CLI, a Hermes Director still receives and answers member
messages, but **neither the messages nor its replies appear in its pane**.
Run the Director in the TUI client: `hermes chat --tui`, or set
`display.interface: tui`. Hermes members keep the classic CLI.
:::

1. Run `cafleet setup --coding-agent hermes` with `hermes` on `PATH`.
2. Start the Director with `hermes chat --tui`.
3. Register the fleet with
   `cafleet fleet create --name <name> --coding-agent hermes`. When CAFleet
   asks for the pane ID and server, run the command it names
   ([Director pane verification](concepts/members.md#director-pane-verification)).
4. Start reception with the plugin's `cafleet_listen` tool, passing the
   Director ID.

### pi

1. Install pi 1.5.0 or later and configure its default provider and model.
2. Run `cafleet setup --coding-agent pi`. Setup installs into `~/.pi/agent`,
   or into `PI_CODING_AGENT_DIR` when you set it; set the same value where
   you run `cafleet member create`.
3. Launch the Director with plain `pi` from the project directory, so the
   CAFleet pi extension loads.
4. Register the fleet with
   `cafleet fleet create --name <name> --coding-agent pi`, then start
   reception with the extension's `cafleet_listen` tool, passing the
   Director ID.
5. Manage a goal with `/cafleet-goal <objective>` and its `status`,
   `pause`, `resume` and `clear` subcommands.

Members run under the CAFleet member gate's `cafleet-ask` mode: their broker
message commands run and every other shell command opens a native permission
form for the Director. Choose `--permission-mode cafleet` for a member whose
other commands should be denied and routed to the Director instead
([pi configuration](concepts/coding-agents.md#pi-configuration)).

### Trust the working directory

A freshly spawned member stalls on its coding agent's first-run trust prompt, ignoring every message until the prompt is cleared. Trust each working directory in advance: launch your coding agent there once and accept the prompt, or add a trust entry to its configuration. For Codex, add the absolute workspace path to `~/.codex/config.toml`:

```toml
[projects."/abs/path/to/workspace"]
trust_level = "trusted"
```

Trust is granted per directory, so each git worktree needs its own approval. pi members skip pi's project-trust prompt by ignoring trust-gated project-local resources, so they need no trust entry.

## Simple example — invoke from a coding agent

Ask your agent: “Use the cafleet skill to create a team with two members, exchange a message, then shut down the team.” The skill manages the bootstrap and supervision protocol; [Run a fleet](mixed-backend-team.md#manual-lifecycle) has the complete command sequence and backend variations.
