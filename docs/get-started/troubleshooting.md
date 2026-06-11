---
icon: lucide/wrench
---

# Troubleshooting

Start with `cafleet doctor`. It prints the calling pane's tmux identifiers
and is the first thing to run on any placement or tmux confusion — a fleet
that will not create, a member command that claims you are outside tmux, a
pane that cannot be found. It requires the `TMUX` and `TMUX_PANE`
environment variables to be set (the standard tmux pane environment):

```bash
cafleet doctor
```

```
tmux:
  session_name:  main
  window_id:     @1
  pane_id:       %0
  TMUX_PANE:     %0
```

## Symptom → fix

| Symptom | Fix |
|---|---|
| `Error: cafleet fleet create must be run inside a tmux session` | Run the command inside tmux; verify the pane environment with `cafleet doctor`. |
| `Error: cafleet member commands must be run inside a tmux session` | Same as above — `member` commands (and `doctor` itself) need the tmux pane environment. |
| `OperationalError: no such table: agents` | The schema was never applied: run `cafleet db init` → [Install](install.md). |
| `Error: DB schema is at revision <X> which is unknown to this version of cafleet. Refusing to downgrade automatically.` (from `cafleet db init`) | Old (UUID-era) database; delete the file and re-run `cafleet db init` → the upgrade warning on [Install](install.md). |
| `Error: --fleet-id <int> is required for this subcommand. Create a fleet with 'cafleet fleet create' and pass its id.` | Do what it says — pass the literal fleet id the create printed → [CLI options](../spec/cli-options.md). |
| `Error: agent <id> is not a member of fleet <id>.` | Wrong `--fleet-id`/`--agent-id` pairing; recover the ids with `cafleet fleet list` (the `DIRECTOR` column carries the Director's id). |
| Permission prompts keep interrupting an agent | Shell variables break `permissions.allow` pattern matching — paste literal integer ids instead → [Configure](configure.md). |
| `Error: binary <name> not found on PATH` | Install the backend binary you asked `member create` to spawn → [Coding agents](../concepts/coding-agents.md). |
| A member never reacts to messages | It missed the inline preview; check `member list --activity`, then `member ping` → [Monitor and recover members](../how-to/monitor-and-recover.md). |
| `Error: pane %N did not close within 15.0s after /exit.` | Follow the recovery hint the command printed (capture → send-input → re-run delete); `member delete --force` as the last resort. |
| WebUI `/` returns 404 | The UI is not built (source checkout): run `mise //admin:build` → [Use the admin WebUI](../how-to/use-the-webui.md). |
| `OSError: [Errno 98] Address already in use` on `cafleet server` | Another process owns the port; pass `--port` → [CLI options](../spec/cli-options.md#cafleet-server). |

Symptom not listed? Check the full
[Error Messages](../spec/cli-options.md#error-messages) table for every
exact error string and exit code.
