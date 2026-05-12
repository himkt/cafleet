---
name: base-dir
description: >
  Resolve the base directory for output files. Loaded by consuming skills
  via Skill(cafleet:base-dir). Do NOT invoke directly.
---

# Base Directory Resolution

## Procedure

1. **Precondition (caller-side, before loading this skill)**: the consuming skill MUST only load this skill when its argument is not an absolute path. For an absolute-path argument, the consuming skill uses the absolute path directly as its own resolved target (e.g., `${DOC_PATH}`, `${FOLDER}`) and does NOT load this skill; `${BASE}` remains undefined in that branch, and any feature that depends on it (such as the audit-file write) is skipped per the consuming skill's own gating rule.
2. If `${CWD} == $HOME` or `${CWD}` is under `$HOME/.claude` (for example, `${CWD}` starts with `$HOME/.claude/`) → go to step 3.
   Otherwise → `${BASE} = ${CWD}`. Done.
3. Ask via `AskUserQuestion` ("Select the base directory for output files:"):
   - `/tmp/claude-code (recommended)` → `${BASE} = /tmp/claude-code`
   - `${CWD}` → `${BASE} = ${CWD}`
   - `Other` (free text) → `${BASE} = user input` (resolve against `${CWD}` if relative)
