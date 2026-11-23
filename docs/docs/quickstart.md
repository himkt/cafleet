# Quickstart

Create a fleet, send a message, and close the team from your coding-agent
pane inside tmux or herdr.

## Install

```bash
brew install himkt/tap/cafleet
```

Alternatively, extract the binary for your platform from [GitHub Releases](https://github.com/himkt/cafleet/releases) onto `PATH`.

Install the native coding agents you intend to use, configure their default
provider/model and complete native authentication. Choose any relocated
[configuration directories](concepts/coding-agents.md#config-directories) before
setup and keep those settings when launching members. Put `hermes` on `PATH`
before setup if you use it.

Run the shared setup once to install embedded skills, presets and plugins offline:

```bash
cafleet setup
```

Rerun `cafleet setup` after every upgrade, patch releases included: each version starts from a fresh database ([Upgrading cafleet](concepts/storage.md#upgrading-cafleet)).

## Configure

Configure the backends you use below. Before unattended member startup, open
the backend in the intended working directory and approve the folder if its
native trust prompt asks ([Working-directory trust](concepts/coding-agents.md#working-directory-trust)).

### Claude Code

Add this recommended `auto` profile to `~/.claude/settings.json`. Auto mode
must be available for your account and model; [Claude Code configuration](concepts/coding-agents.md#claude-code-configuration)
covers other permission modes.

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

Restart Codex after setup so it loads `~/.codex/rules/cafleet.rules`.
Keep your own rules in a separate file beside it; setup replaces its installed
file on every run.

If the Director's current policy prevents needed escalation, add these top-level keys to
`~/.codex/config.toml`, before any table headers:

```toml
approval_policy = "on-request"
sandbox_mode = "workspace-write"
```

Keep an existing automatic reviewer that supports the needed escalation.
[Codex configuration](concepts/coding-agents.md#codex-configuration) explains
reviewer selection and conditional sandbox grants.

Registration may ask for the Director's pane ID and server
([Director pane verification](concepts/members.md#director-pane-verification)).

### OpenCode

Use OpenCode 2.2.0 or later and launch the Director with
`opencode mini --standalone` from the project directory, with the default agent.
[OpenCode configuration](concepts/coding-agents.md#opencode-configuration) covers
member permissions and the installed plugin.

### Hermes

:::warning Start the Director with `hermes chat --tui`
Previews of member messages are typed only into the Hermes TUI client, so a Director
in the classic CLI **receives no previews in its pane** and reads messages
only by manual poll. Run the Director in the TUI client: `hermes chat --tui`,
or set `display.interface: tui`. Hermes members keep the classic CLI.
:::

Keep `approvals.mode` at its `smart` default.
[Hermes configuration](concepts/coding-agents.md#hermes-configuration) covers troubleshooting.

### pi

Use pi 1.5.0 or later and launch the Director with plain `pi` from the project
directory. Setup installs into `~/.pi/agent`, or `PI_CODING_AGENT_DIR` when set;
use the same value where you run `cafleet member create`, which forwards it to
members. [pi configuration](concepts/coding-agents.md#pi-configuration) covers
the extension and member permissions.

## Simple example — invoke from a coding agent

Ask your agent: “Use the cafleet skill to create a team with two members, exchange a message, then shut down the team.” The skill manages the bootstrap and supervision protocol; [Run a fleet](mixed-backend-team.md#manual-lifecycle) has the complete command sequence and backend variations.
