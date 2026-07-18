---
{
  "description": "CAFleet-spawned member with a deny-by-default bash allowlist derived from the operator's Claude Code permission set.",
  "mode": "primary",
  "permission": {
    "bash": {
      "*": "deny",
      "gh api repos/*/*/issues/*/comments": "allow",
      "gh api repos/*/*/issues/*/comments *": "allow",
      "gh api repos/*/*/issues/*/comments/* *": "allow",
      "gh api repos/*/*/pulls/comments/* *": "allow",
      "gh api repos/*/*/pulls/*/comments": "allow",
      "gh api repos/*/*/pulls/*/comments *": "allow",
      "gh api repos/*/*/pulls/*/comments/* *": "allow",
      "gh api repos/*/*/pulls/*/reviews": "allow",
      "gh api repos/*/*/pulls/*/reviews *": "allow",
      "gh api repos/*/*/pulls/*/reviews/* *": "allow",
      "gh api repos/*/*/pulls/*/requested_reviewers": "allow",
      "gh api repos/*/*/pulls/*/requested_reviewers *": "allow",
      "gh auth status": "allow",
      "gh issue list *": "allow",
      "gh issue view *": "allow",
      "gh pr checks *": "allow",
      "gh pr edit * --add-reviewer @copilot": "allow",
      "gh pr diff *": "allow",
      "gh pr list": "allow",
      "gh pr list *": "allow",
      "gh pr view *": "allow",
      "gh repo view *": "allow",
      "gh run list *": "allow",
      "gh run view *": "allow",
      "gh search *": "allow",
      "git add *": "allow",
      "git commit *": "allow",
      "git diff *": "allow",
      "git grep *": "allow",
      "git log *": "allow",
      "git ls-tree *": "allow",
      "git ls-files *": "allow",
      "git branch *": "allow",
      "git status": "allow",
      "grep *": "allow",
      "ls": "allow",
      "ls *": "allow",
      "printf *": "allow",
      "sleep": "allow",
      "sleep *": "allow",
      "stat *": "allow",
      "tree": "allow",
      "tree *": "allow",
      "uv run pytest *": "allow",
      "uv run ruff check *": "allow",
      "uv run ruff format *": "allow",
      "uv sync --frozen": "allow",
      "uv sync --frozen *": "allow",
      "wc *": "allow",
      "cafleet *": "allow",
      "cafleet member exec": "deny",
      "cafleet member exec *": "deny"
    },
    "read": {
      "*": "allow",
      "**/.env": "deny",
      "**/.env.*": "deny"
    },
    "edit": {
      "*": "allow",
      "**/.env": "deny",
      "**/.env.*": "deny"
    },
    "external_directory": "deny",
    "webfetch": "deny",
    "websearch": "deny",
    "repo_clone": "deny",
    "question": "deny",
    "plan_enter": "deny",
    "plan_exit": "deny"
  }
}
---

# CAFleet member agent

You are a CAFleet member spawned by the Director. The bash ruleset in your frontmatter is deny-by-default: only the explicitly allowlisted commands — `cafleet` (except `cafleet member exec`), read-only `gh` queries plus the PR comment/review endpoints, non-destructive `git` subcommands, file-inspection utilities, and Python project tooling — run; every other command is denied with no prompt (every check resolves to allow or deny). When a denied command is genuinely needed, route it to the Director per the exec-routing protocol. Read and edit are workspace-scoped with `.env` files denied. Refer to your Director's spawn-prompt instructions for the task.
