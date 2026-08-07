# Overlay: claude

Substitute these into the base `{…}` placeholders.

| Placeholder | Value |
|---|---|
| `{decision_surface}` | the AskUserQuestion tool |
| `{reviewer_model}` | `fable` |
| `{permission_flags}` | `--permission-mode dontAsk` |
| `{bg_run}` | the Bash tool's `run_in_background: true` |
| `{bg_stop}` | `TaskStop` |
| `{task_coord}` | the harness task list |
| `{pane_title}` | `claude --name <member-name>` sets `#{pane_title}` to the member name |
| `{skill_loader}` | the Skill tool (dispatch sub-agents via the Agent tool) |
| `{effort_levels}` | `low`, `medium`, `high`, `xhigh`, `max` (spawn flag `--effort <level>`) |

## Note → applies at

Every note names the base token/instruction it qualifies.

| Note | Applies at |
|------|-----------|
| `AskUserQuestion` takes ≤ 4 options/question; the built-in "Other" is the free-text path (don't add an explicit "Other"). Question shapes → form: choice among ≤ 4 labeled options; approve-or-revise (two options); continue-or-abort (two options); open-ended draft-comparison (2–4 full candidate bodies). | `{decision_surface}` — `cafleet/SKILL.md` § Soliciting user reactions; `cafleet-design-doc/create/create.md` Step 2 question batch |
| `TaskCreate` / `TaskUpdate` / `TaskList` / `TaskGet`: register a sub-topic with `TaskCreate`, claim with `TaskUpdate` (owner + `in_progress`), complete with `TaskUpdate` (`completed`), check progress with `TaskList`. | `{task_coord}` — `cafleet-research/report/report.md` task coordination |
| *Pane-state capture cues* (below) — the concrete claude-pane discriminators for `awaiting_user`, `finished`, affirmative `working`, and quiet `stall_candidate`. | the Director's on-tick health check and pre-ping capture gate — `cafleet/reference/supervision.md` § Idle Semantics / § Stall Response; the pane-state taxonomy in `docs/docs/concepts/monitoring.md` (the Director applies the cues of the **target member's** backend overlay). |

## Pane-state capture cues

The Director classifies each target from its **content only** (never native `agent_status`) using that target's backend overlay:

| State | claude capture cue |
|---|---|
| `awaiting_user` | A bordered prompt box awaiting a keypress: an `AskUserQuestion` selection list (numbered options with a `❯`-marked choice and an "Other" free-text entry) or a tool/permission approval prompt (`Do you want to proceed?` with `❯ 1. Yes` / `2. No`). A choice is pending; the composer is not at rest. |
| `finished` | The bare composer at rest: an empty `>` input line with its placeholder hint and the idle status line beneath (model + context, the `⏵⏵` mode indicator), with **no** question/approval box above it and **no** running spinner or `esc to interrupt` indicator. |
| `working` | Affirmative active-work evidence: a running spinner, `esc to interrupt`, streaming response, tool execution, or generation indicator. A truncated or ambiguous capture that might hide one of these cues is also `working`. |
| `stall_candidate` | Quiet, non-finished transcript content with no question/approval box, no empty at-rest composer, and no spinner, tool, streaming, generation, or other active-work cue. |

Tie-breaks and the two-quiet-families rule: `supervision.md` § *The pre-ping capture gate*.

## Worked resolution

The canonical Director-side monitor launch, fully resolved for this backend:

Launch `cafleet monitor <fleet-id>` via the Bash tool with `run_in_background: true`, and confirm `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` in the task output before the first `member create` (members spawned `--permission-mode dontAsk`).
