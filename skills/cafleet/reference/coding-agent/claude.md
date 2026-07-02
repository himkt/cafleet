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

## Note → applies at

Every note names the base token/instruction it qualifies.

| Note | Applies at |
|------|-----------|
| `AskUserQuestion` takes ≤ 4 options/question; the built-in "Other" is the free-text path (don't add an explicit "Other"). Question shapes → form: choice among ≤ 4 labeled options; approve-or-revise (two options); continue-or-abort (two options); open-ended draft-comparison (2–4 full candidate bodies). | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions; `cafleet-design-doc/create/create.md` Step 2 question batch |
| `TaskCreate` / `TaskUpdate` / `TaskList` / `TaskGet`: register a sub-topic with `TaskCreate`, claim with `TaskUpdate` (owner + `in_progress`), complete with `TaskUpdate` (`completed`), check progress with `TaskList`. | `{task_coord}` — `cafleet-research/report/report.md` task coordination |

## Worked resolution

The canonical monitor-spawn command, fully resolved for this backend:

`cafleet agent spawn --role monitor --model haiku --text-file <rendered monitor prompt>` (members spawned `--permission-mode dontAsk`).
