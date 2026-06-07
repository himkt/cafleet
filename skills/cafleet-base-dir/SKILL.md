---
name: cafleet-base-dir
description: >
  Resolve the base directory for output files. Loaded by consuming skills
  via the `cafleet-base-dir` skill. Do NOT invoke directly.
---

# Base Directory Resolution

The `cafleet-base-dir` skill is the single authoritative resolver for every CAFleet scratch / audit / figure path. The resolution outcome (`${BASE}`) is the only legitimate root for those writes. Consuming skills MUST NOT compute `${BASE}` independently and MUST NOT fall back to `/tmp` when resolution returns the `<unset>` sentinel.

`/tmp/claude-code` remains a perfectly valid resolved `${BASE}` when this skill explicitly selects it via the `AskUserQuestion` branch — only **bypassing** base-dir to write to `/tmp` without its consent is forbidden.

## Procedure

This skill resolves `${BASE}` using only `git rev-parse --show-toplevel` (Bash) and `AskUserQuestion`. It writes nothing at resolution time. Claude's job is to (a) pick the task-scope branch (Step 0) when the consuming skill operates on a per-task folder, otherwise the shared-root branch (Step 1); and (b) drive `AskUserQuestion` (Step 2) only when Step 1 ends in "needs user input".

### Step 0. Task-scope resolution (preferred for task-aware consuming skills)

When the consuming skill operates on a per-task folder, it picks the task-relpath itself (the resolver is general-purpose and does NOT enumerate or special-case any bucket name — `researches/`, `design-docs/`, etc. are all consumer-side conventions). Resolve as follows:

1. **Infer the repo root.** Run `git rev-parse --show-toplevel` (Bash) from the CWD. A non-zero exit or empty output means "no `.git` ancestor" → STOP and tell the user to `cd` to the repo root and retry.
2. **Canonicalize** `$ARGUMENTS` to the task-folder relpath per the consumer's convention (table below) BEFORE proceeding.
3. **Guard the path.** Reject a `task_name` that escapes or degenerates to the repo root:
   - **Relative `task_name`**: join it under the repo root and resolve. If the result equals the repo root → STOP ("the repo root is not a task folder"). If the result is not under the repo root (a `..` escape) → STOP ("refusing to create a task folder outside the repo").
   - **Absolute `task_name`**: resolve it. If it equals the repo root OR is not strictly under it → `${BASE} = <unset>` (the absolute-path-arg branch); create nothing.
4. `${BASE}` = the absolute task folder. The folder is created lazily on the first consumer write (the `Write` tool auto-creates parent directories), so there is no resolution-time write. Audit files such as `${BASE}/prompts/<role>-<UTC-compact>.md` land under the task folder rather than at the repo root.

**The consuming skill is responsible for passing a relpath that is the actual task folder**, not a child file path — this procedure does no slug-folding or filename-stripping.

**Consumer contract — canonicalize ARGUMENTS to the task-folder relpath before resolving.** The procedure is deliberately general: it does not strip trailing filenames (e.g. `/design-doc.md`, `/report.md`) and does not strip leading bucket prefixes (e.g. `design-docs/`, `researches/`). It creates `<task-folder>/` exactly as supplied. Each consuming skill MUST canonicalize `$ARGUMENTS` against its own convention BEFORE running Step 0:

| Consumer | Canonical relpath form | Canonicalization steps |
|:--|:--|:--|
| The `cafleet-design-doc-create` / `cafleet-design-doc-execute` / `cafleet-design-doc-interview` skills | `design-docs/<slug>` | (1) strip trailing `/design-doc.md` if present; (2) strip leading `design-docs/` if present; (3) prepend `design-docs/`. **Absolute paths**: apply the same `/design-doc.md` strip (a child file path otherwise becomes a directory named after the file). |
| The `cafleet-research-report` / `cafleet-research-presentation` skills | `researches/<topic-slug>` | (1) strip trailing `/report.md` (or other known per-topic filenames) if present; (2) strip leading `researches/` if present; (3) prepend `researches/`. **Absolute paths**: apply the same `/report.md` strip (a child file path otherwise becomes a directory named after the file). |

The stripping logic is skill-specific because each skill knows the file conventions inside its own bucket. The resolution procedure stays bucket-agnostic. Skipping canonicalization creates a directory literally named `design-doc.md` (or `report.md`, etc.) and resolves the wrong BASE.

### Step 1. Shared-root resolution (no task name)

When the consuming skill has no per-task folder convention (the shared-root case):

1. Determine the CWD and `$HOME`. Let `claude_subdir = $HOME/.claude`.
2. **If the CWD is NOT `$HOME` and NOT `claude_subdir` (nor under it)** → `${BASE}` is the CWD itself (the working directory, **not** the repo root). Done.
3. **Otherwise** (CWD is `$HOME` or under `~/.claude`) → go to Step 2 with candidates `[/tmp/claude-code, <CWD>]`.

### Step 2. AskUserQuestion (only when Step 1 ends in "needs user input")

Present the candidates via `AskUserQuestion` ("Select the base directory for output files:"):

- `/tmp/claude-code (recommended)` → `${BASE} = /tmp/claude-code`
- `${CWD}` → `${BASE} = ${CWD}` (the CWD literal from the candidates list)
- `Other` (free text) → `${BASE} = user input` (resolve against `${CWD}` if relative)

The chosen value is `${BASE}`. Nothing is persisted; if the skill reloads in the same `$HOME` / `~/.claude` CWD, this branch re-prompts.

## No-bypass write protocol

Every CAFleet member, every consumer skill, and every Director MUST follow this protocol for scratch / audit / figure / spawn-prompt-render writes:

1. **Every write under `${BASE}` or an explicit consumer-supplied absolute target.** Scratch (pre-spawn renders of spawn prompts at `${BASE}/prompts/<role>-<UTC-compact>.md`, working notes), audit files, figure artifacts, and any other ephemeral output MUST land under `${BASE}` or under a consumer-supplied absolute path — e.g., the design-doc directory delivered to spawned members via `[INSERT abs design-doc directory]`, or the research folder delivered via `[INSERT abs research folder]`. Never `/tmp` unless `${BASE}` itself is `/tmp/claude-code` (which is a legitimate base-dir choice).

2. **`${BASE} == <unset>` is a hard stop, not a fallback.** If `${BASE}` is the literal sentinel `<unset>` (absolute-path argument branch), any code that tries to compute a path from `${BASE}` MUST abort with `Error: BASE is <unset>; refusing to fall back to /tmp`. The loud failure is the safety net for sites that forgot to guard explicitly.

3. **Members never re-resolve BASE.** The Director's spawn-prompt substitution delivers `${BASE}` to each spawned member as a literal absolute path baked into the spawn prompt. Members MUST use that literal path verbatim. Members DO load this skill at startup (per their role file's *Load at Startup* block) to pick up the no-bypass write protocol and the `<unset>` sentinel contract — but they MUST NOT run the resolution procedure (Steps 0–2) or otherwise derive a new `${BASE}` of their own. Re-resolving would invite drift if the Director's resolved BASE changed mid-session.

4. **Missing-BASE-line anchorless status.** If a member's spawn prompt is missing the `BASE:` line entirely (an expected outcome when the Director resolved `${BASE} = <unset>`), the member treats the audit-file feature as disabled and emits a single CAFleet message back to the Director as a parens-free anchorless status (per `skills/cafleet-design-doc/coordination.md` § *Anchorless Status*):

   ```
   audit-disabled no BASE in spawn prompt
   ```

   The phrasing deliberately omits parentheses so a Director reading the broker log does not misinterpret it as a malformed `<verb> (<pointer>)` hop. The member MUST NOT fall back to `/tmp`.

## The `<unset>` sentinel

The `cafleet-base-dir` skill is the only place that may return `BASE=<unset>`. The sentinel is the literal string `"<unset>"` (case-sensitive). Any consumer skill that has `${BASE}` set to `<unset>` MUST:

1. **Guard audit-file writes.** Audit-file writes that derive their path from `${BASE}` are guarded by an explicit `${BASE} != <unset>` check at the call site. The guard is mandatory; reaching `Path(BASE) / …` without the guard is the failure mode item 3 catches. A guarded skip is the intended path when `${BASE}` is `<unset>` — no silent no-op, no fallback to `/tmp`, just an explicit skip whose condition is visible in the source.
2. **Omit `BASE:` from spawn prompts.** The Director MUST NOT spawn members with `BASE: <unset>` in the spawn prompt — the line is omitted entirely so the member's existence-check naturally treats audit-file features as disabled. The literal string `BASE: <unset>` is never written into any spawn prompt.
3. **Loud-error on unguarded BASE-derivation.** If a code path under `${BASE}=<unset>` reaches an unguarded `Path(BASE) / …` computation, abort with the standardized error: `Error: BASE is <unset>; refusing to fall back to /tmp`.

Skip (item 1) and loud-error (item 3) are different stages of the same protocol, not alternative resolutions: every BASE-dependent write site MUST be guarded; the loud error is the safety net for sites that forgot to guard.
