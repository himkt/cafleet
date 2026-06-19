# Overlay: claude

Substitute these into the base `{…}` placeholders.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | `AskUserQuestion` |
| `{monitor_model}` | `haiku` |
| `{member_model}` | `sonnet` |
| `{permission_flags}` | `--permission-mode dontAsk` |
| `{bg_run}` | the Bash tool's `run_in_background: true` |
| `{bg_stop}` | `TaskStop` |
| `{task_coord}` | the harness task list (`TaskCreate` / `TaskUpdate` / `TaskList` / `TaskGet`) |
| `{pane_title}` | `claude --name <member-name>` sets `#{pane_title}` to the member name |
| `{skill_loader}` | the Skill tool (dispatch sub-agents via the Agent tool) |

Decision-surface constraints: `AskUserQuestion` takes ≤ 4 options/question; the built-in "Other" is the free-text path (don't add an explicit "Other"); a standalone agent calls it directly, a fleet member routes its question to the Director, which relays it.

Relaying a member's question: ask the user, then forward via `cafleet member send-input --choice N | --freetext`; keystrokes: `docs/spec/cli-options.md#member-send-input`.
