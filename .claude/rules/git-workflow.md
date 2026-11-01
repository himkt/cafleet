# Git Workflow (Project-Specific Overrides)

This file overrides the host's Git Workflow rule for this project only. The host rule still applies except where explicitly contradicted here.

## design-docs/ is committed in this project

**Override**: Commit only `design-doc.md` from each design folder as part of the feature branch that implements it.

- Stage `design-docs/NNNNNNN-<slug>/design-doc.md` alongside the implementation commits.
- A dedicated `docs: add design doc NNNNNNN-<slug>` commit at the end of the implementation sequence is acceptable, as is including the design doc in the first implementation commit.
- The repository's `.gitignore` keeps every other path inside a design folder local, including prompts, audits, screenshots, scans and clarification files.
- `researches/` remains gitignored and local.
