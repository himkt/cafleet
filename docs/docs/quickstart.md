# Quickstart

Create a fleet, send a message, and close the team from your coding-agent
pane inside tmux or herdr.

## Install

```bash
brew install himkt/tap/cafleet
```

Alternatively, extract the binary for your platform from [GitHub Releases](https://github.com/himkt/cafleet/releases) onto `PATH`.

Install the native coding agents you intend to use and choose any relocated
[configuration directories](concepts/coding-agents.md#config-directories) before
setup. Keep those directory settings when launching members. If you use Hermes,
make `hermes` available on `PATH` so setup installs and enables its plugin;
[Hermes troubleshooting](concepts/coding-agents.md#hermes-configuration) covers
installing it later.

Run the shared setup once to install embedded skills, presets and plugins offline:

```bash
cafleet setup
```

Rerun `cafleet setup` after every upgrade, patch releases included: each version starts from a fresh database ([Upgrading cafleet](concepts/storage.md#upgrading-cafleet)).

## Configure

After shared setup, configure each backend you use:

| Backend | Config file | Manual configuration | Installed by `cafleet setup` |
|---|---|---|---|
| [`claude` (Claude Code)](#claude-code) | `~/.claude/settings.json` | The recommended `auto` permission profile below | The skills |
| [`codex` (OpenAI Codex CLI)](#codex) | `~/.codex/config.toml` | A Director approval policy that permits needed escalation | The skills, plus `~/.codex/rules/cafleet.rules` |
| [`opencode`](#opencode) | — | — | The skills, the `cafleet` and `cafleet-ask` agent presets and the CAFleet plugin |
| [`hermes` (Hermes Agent)](#hermes) | `~/.hermes/config.yaml` | Keep `approvals.mode` at its `smart` default | The skills and the CAFleet plugin, enabled in your Hermes configuration |
| [`pi` (pi coding agent)](#pi) | — | — | The skills and the CAFleet pi extension |

The paths above are defaults ([Config directories](concepts/coding-agents.md#config-directories) covers relocating them); [Backend configuration](concepts/coding-agents.md#backend-configuration) has each backend's full reference and troubleshooting.

### Claude Code

Add the recommended `auto` profile to `~/.claude/settings.json`. Members start
in `auto` mode by default, so auto mode must be available for your account and
model. [Claude Code configuration](concepts/coding-agents.md#claude-code-configuration)
covers other supported permission modes.

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

The optional `ask` entries let you confirm each input the Director sends into
a member's pane; drop them to let it dispatch without a prompt. Add an allow
entry, such as `Bash(npm test *)`, for every project command members run.

### Codex

Restart Codex after shared setup so it loads `~/.codex/rules/cafleet.rules`.
Keep your own rules in a separate file beside it; setup replaces its installed
file on every run.

Before unattended member startup, approve the repository or directory shown by
Codex's native `Trust this folder?` prompt. Codex 0.167.0 requests this consent
for a fresh repository and a new worktree on the tested Linux launch;
[working-directory trust](concepts/coding-agents.md#working-directory-trust)
explains the scope.

For a Director that needs to request escalation, add these top-level keys to
`~/.codex/config.toml`, before any table headers:

```toml
approval_policy = "on-request"
sandbox_mode = "workspace-write"
```

Keep an existing automatic reviewer when it already supports the needed
escalation. The approval policy and reviewer are separate settings; this example
selects the Director's policy, while `cafleet member create --permission-mode`
selects a spawned member's policy. [Codex configuration](concepts/coding-agents.md#codex-configuration)
explains the reviewer and any sandbox access needed for your command path.

The Director's listener must emit `ready`. If the sandbox blocks startup, for
example with `cannot run ps: Operation not permitted (os error 1)` on macOS,
follow [Codex listener troubleshooting](concepts/coding-agents.md#codex-listener-startup)
to retry through the host's approval flow and confirm notification delivery.

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

1. Start the Director with `hermes chat --tui`.
2. Register the fleet with
   `cafleet fleet create --name <name> --coding-agent hermes`. When CAFleet
   asks for the pane ID and server, run the command it names
   ([Director pane verification](concepts/members.md#director-pane-verification)).
3. Start reception with the plugin's `cafleet_listen` tool, passing the
   Director ID.

### pi

Use pi 1.5.0 or later. Shared setup installs into `~/.pi/agent`, or into
`PI_CODING_AGENT_DIR` when you set it; use the same value where you run
`cafleet member create`.

1. Configure pi's default provider and model.
2. Launch the Director with plain `pi` from the project directory, so the
   CAFleet pi extension loads.
3. Register the fleet with
   `cafleet fleet create --name <name> --coding-agent pi`, then start
   reception with the extension's `cafleet_listen` tool, passing the
   Director ID.
4. Manage a goal with `/cafleet-goal <objective>` and its `status`,
   `pause`, `resume` and `clear` subcommands.

Members run under the CAFleet member gate's `cafleet-ask` mode: their broker
message commands run and every other shell command opens a native permission
form for the Director. Choose `--permission-mode cafleet` for a member whose
other commands should be denied and routed to the Director instead
([pi configuration](concepts/coding-agents.md#pi-configuration)).

## Simple example — invoke from a coding agent

Ask your agent: “Use the cafleet skill to create a team with two members, exchange a message, then shut down the team.” The skill manages the bootstrap and supervision protocol; [Run a fleet](mixed-backend-team.md#manual-lifecycle) has the complete command sequence and backend variations.
