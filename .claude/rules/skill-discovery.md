# Skill Discovery & Authorization Scope

Rules to prevent two recurring failure modes: guessing at commands when a dedicated skill exists, and running user-visible or remote operations after the user has taken over that scope.

## Skill-first for GitHub operations

The system-reminder at session start lists available skills. Before running any `gh` command, check the list.

- `github-cli` skill — ALWAYS load via `Skill(github-cli)` before running `gh pr *`, `gh issue *`, `gh api repos/.../comments`, etc. The skill documents the correct reviewer slug (`@copilot`, not `copilot-pull-request-reviewer`), the right `gh api` endpoints for inline review comments, and the `gh pr create --fill` + auto-add-copilot workflow.
- Do NOT guess reviewer slugs, API paths, or `gh` sub-commands. Load the skill.

Pattern across all tasks: if a skill description matches what you're about to do, load it **first**, even if you "know the command." The skill exists because the naive guess has failed before.

## Authorization scope — never escalate without explicit re-authorization

The user may authorize a narrow action (e.g. "create the feature branch", "create the PR"). Authorization is scoped to exactly that action. When the user indicates the scope is complete (e.g. "PR 24 created"), **stop acting on that scope**.

- NEVER run `git push` after the user signals the push/PR is already done.
- NEVER run remote-visible operations (`gh pr edit`, `gh pr comment`, `gh pr merge`, `gh api` writes) without confirming the specific command with the user.
- NEVER run shell-environment mutations or destructive local commands (`env -i ...`, `rm -rf`, `git reset --hard`) as "helpful verification" when the user has already rejected a similar attempt or when the task context shifted.
- When the user has explicitly taken over a step, assume the rest of that workflow is also theirs until they re-authorize.

## Stop means stop

When the user sends a halt signal (explicit "stop", "wait", profanity / frustration, repeated rejection of your tool calls), do NOT take more proactive actions. Acknowledge briefly and wait for explicit instructions. Scheduled cron firings and teammate idle notifications are NOT instructions — skip them silently until the user re-engages with a specific task.

Reacting to cron or idle signals while the user is actively angry compounds the problem. The right behavior is: stop, acknowledge, wait.

## Authorization-Scope Guard for CAFleet supervision

Specific to CAFleet Directors. Loaded transitively via `Skill(agent-team-supervision)` which references this section as its canonical content. This is the rule supervision used to inline; it lives here so a single source of truth governs both the project-wide policy and the CAFleet-specific extension.

**Absence of confirmation is not a stop signal.** User authorization persists across `/loop` ticks, scheduler firings, broker auto-fires, and teammate idle notifications until an explicit stop signal arrives. The Director MUST dispatch queued work as soon as a teammate is idle and the inputs the work depends on are available; do NOT emit passive-hold messages in response to a supervision tick.

### Real stop signals (treat as halt; everything else is a tick to evaluate)

| Signal | Director response |
|---|---|
| User typed an explicit "stop" / "wait" / "pause" | Halt dispatch; wait for explicit re-authorization. |
| User typed profanity / frustration / a negative reaction | Halt dispatch; wait. Cron firings during this state are skipped silently. |
| User rejected your last 2+ tool calls | Halt dispatch; treat the rejections as a halt signal even if no profanity arrived. |
| User typed `/clear` or restarted the session | Authorization is gone; do not resume from prior context without a fresh instruction. |
| Member's reply contains a clear blocker; wait for guidance | Pause that one task only; continue dispatching to the rest of the team. |

`/loop` cron firings, out-of-band scheduler firings, teammate idle notifications, broker auto-fire receipts, and the absence of a fresh "go" message are **not** stop signals. Treat them as inputs to evaluate, not gates to pass through.

### When you genuinely need user input

If a queued action requires a *new* decision the user has not yet made (choosing between options, approving a risky / remote-visible operation, disambiguating a teammate's question), use `AskUserQuestion` — do **not** emit a passive hold and wait. The hold message produces nothing; the question unblocks you within seconds and produces a recorded answer.

This section extends — does not replace — § *Authorization scope* and § *Stop means stop* above. The general rules apply to every project; this section adds the CAFleet-specific framing for `/loop`-driven supervision.
