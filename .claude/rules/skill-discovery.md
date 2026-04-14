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
