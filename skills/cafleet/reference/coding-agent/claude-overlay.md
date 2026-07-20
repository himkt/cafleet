# Overlay: claude

Substitute these into the base `{…}` placeholders.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the AskUserQuestion tool |
| `{permission_flags}` | `--permission-mode dontAsk` |
| `{bg_run}` | the Bash tool's `run_in_background: true` |
| `{bg_stop}` | `TaskStop` |
| `{task_coord}` | the harness task list |
| `{pane_title}` | `claude --name <member-name>` sets `#{pane_title}` to the member name |
| `{skill_loader}` | the Skill tool (dispatch sub-agents via the Agent tool) |
| `{effort_levels}` | `low`, `medium`, `high`, `xhigh`, `max` (spawn flag `--effort <level>`) |
| `{monitor_model}` | `claude-haiku-4-5` |
| `{reviewer_model}` | `claude-fable-5` |

## Note → applies at

Every note names the base token/instruction it qualifies.

| Note | Applies at |
|------|-----------|
| `AskUserQuestion` takes ≤ 4 options/question; the built-in "Other" is the free-text path (don't add an explicit "Other"). Question shapes → form: choice among ≤ 4 labeled options; approve-or-revise (two options); continue-or-abort (two options); open-ended draft-comparison (2–4 full candidate bodies). | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions; `cafleet-design-doc/create/create.md` Step 2 question batch |
| `TaskCreate` / `TaskUpdate` / `TaskList` / `TaskGet`: register a sub-topic with `TaskCreate`, claim with `TaskUpdate` (owner + `in_progress`), complete with `TaskUpdate` (`completed`), check progress with `TaskList`. | `{task_coord}` — `cafleet-research/report/report.md` task coordination |
| *Pane-state capture cues* (below) — the concrete claude-pane discriminators for `awaiting_user` vs `finished`. | classification rubric rule 1 (`awaiting_user`) — `cafleet/roles/monitor.md` § On each wake (step 2); the pane-state taxonomy in `docs/concepts/monitoring.md`; the Director's pre-nudge capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response (the Director applies the cues of the **target member's** backend overlay). |

## Pane-state capture cues

The monitoring member classifies each captured pane from its **content only** (never native `agent_status`). These are the claude-backend discriminators for the two states a single capture most easily confuses — `awaiting_user` (rule 1, the destructive-if-missed class) vs `finished` (rule 3):

| State | claude capture cue |
|---|---|
| `awaiting_user` | A bordered prompt box awaiting a keypress: an `AskUserQuestion` selection list (numbered options with a `❯`-marked choice and an "Other" free-text entry) or a tool/permission approval prompt (`Do you want to proceed?` with `❯ 1. Yes` / `2. No`). A choice is pending; the composer is not at rest. |
| `finished` | The bare composer at rest: an empty `>` input line with its placeholder hint and the idle status line beneath (model + context, the `⏵⏵` mode indicator), with **no** question/approval box above it and **no** running spinner or `esc to interrupt` indicator. |

When a capture shows neither cleanly (e.g. a box scrolled partly out of the capture window), apply the ambiguity tie-break and classify `awaiting_user`.

## Worked resolution

The canonical monitor-spawn command, fully resolved for this backend:

`cafleet member create --role monitor --coding-agent claude --model claude-haiku-4-5 --text-file <rendered monitor prompt>` (members spawned `--permission-mode dontAsk`).
