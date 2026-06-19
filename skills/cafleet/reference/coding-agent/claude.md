# Overlay: claude

Substitute these into the base `{…}` placeholders.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the AskUserQuestion tool |
| `{monitor_model}` | `haiku` |
| `{permission_flags}` | `--permission-mode dontAsk` |
| `{bg_run}` | the Bash tool's `run_in_background: true` |
| `{bg_stop}` | `TaskStop` |
| `{task_coord}` | the harness task list |
| `{pane_title}` | `claude --name <member-name>` sets `#{pane_title}` to the member name |
| `{skill_loader}` | the Skill tool (dispatch sub-agents via the Agent tool) |

Decision surface: `AskUserQuestion` takes ≤ 4 options/question; the built-in "Other" is the free-text path (don't add an explicit "Other"). Question shapes → AskUserQuestion form: choice among ≤ 4 labeled options; approve-or-revise (two options); continue-or-abort (two options); open-ended draft-comparison (2–4 full candidate bodies). A standalone agent calls it directly; a fleet member routes its question to the Director, which relays it.

Relaying a member's question: ask the user, then forward via `cafleet member send-input --choice N | --freetext`; keystrokes: `docs/spec/cli-options.md#member-send-input`.

Task coordination: the harness `TaskCreate` / `TaskUpdate` / `TaskList` / `TaskGet` tools — register a sub-topic with `TaskCreate`, claim with `TaskUpdate` (set owner + `in_progress`), complete with `TaskUpdate` (`completed`), check progress with `TaskList`.
