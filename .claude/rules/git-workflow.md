# Git Workflow (Project-Specific Overrides)

This file overrides the user's global `~/.claude/rules/git-workflow.md` for this project only. The global rules still apply except where explicitly contradicted here.

## design-docs/ is committed in this project

**Override**: The global rule says `NEVER commit files from design-docs/`. In this project, design docs **MUST** be committed as part of the same feature branch that implements them.

- Stage `design-docs/NNNNNNN-<slug>/design-doc.md` alongside the implementation commits.
- A dedicated `docs: add design doc NNNNNNN-<slug>` commit at the end of the implementation sequence is acceptable, as is including the design doc in the first implementation commit.
- Do **not** rely on the user's global gitignore — the file is tracked here.
- `researches/` (the other half of the global rule) remains gitignored and is NOT committed.
