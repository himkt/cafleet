# Base Directory Resolution

Read this file for the base-directory resolution procedure and the no-bypass write protocol. This procedure is the single authoritative resolver for every CAFleet scratch / audit / figure path. The resolution outcome (`${BASE}`) is the only legitimate root for those writes. Consuming skills MUST NOT compute `${BASE}` independently and MUST NOT fall back to `/tmp` when resolution returns the `<unset>` sentinel.

`/tmp/cafleet` remains a perfectly valid resolved `${BASE}` when this procedure explicitly selects it via the `{decision_surface}` branch — only **bypassing** base-dir to write to `/tmp` without its consent is forbidden.

## Procedure

This procedure resolves `${BASE}` using only `git rev-parse --show-toplevel` (Bash) and `{decision_surface}`. It writes nothing at resolution time. The resolving agent's job is to (a) pick the task-scope branch (Step 0) when the consuming skill operates on a per-task folder, otherwise the shared-root branch (Step 1); and (b) drive the `{decision_surface}` prompt (Step 2) only when Step 1 reaches branch 3 (CWD is `$HOME` or under a coding agent's user-level config directory).

### Step 0. Task-scope resolution (preferred for task-aware consuming skills)

When the consuming skill operates on a per-task folder, it picks the task-folder path itself (the resolver is general-purpose and does NOT enumerate or special-case any bucket name — `researches/`, `design-docs/`, etc. are all consumer-side conventions). Resolve as follows:

1. **Infer the repo root.** Run `git rev-parse --show-toplevel` (Bash) from the CWD. A non-zero exit or empty output means "no `.git` ancestor" → STOP and tell the user to `cd` to the repo root and retry.
2. **Canonicalize** `$ARGUMENTS` to the task-folder path per the consumer's convention (table below) BEFORE proceeding. The task-folder path may be relative or absolute.
3. **Guard the task-folder path.** Reject one that escapes or degenerates to the repo root:
   - **Relative task-folder path**: join it under the repo root and resolve. If the result equals the repo root → STOP ("the repo root is not a task folder"). If the result is not under the repo root (a `..` escape) → STOP ("refusing to create a task folder outside the repo").
   - **Absolute task-folder path**: resolve it. If it equals the repo root OR is not strictly under it → `${BASE} = <unset>` (the absolute-path-arg branch); create nothing.
4. `${BASE}` = the absolute task folder. The folder is created lazily on the first consumer write (the `Write` tool auto-creates parent directories), so there is no resolution-time write. Audit files such as `${BASE}/.prompts/<role>-<UTC-compact>.md` land under the task folder rather than at the repo root.

**The consuming skill is responsible for passing a task-folder path that is the actual task folder**, not a child file path — this procedure does no slug-folding or filename-stripping.

**Consumer contract — canonicalize ARGUMENTS to the task-folder path before resolving.** The procedure is deliberately general: it does not strip trailing filenames (e.g. `/design-doc.md`, `/report.md`) and does not strip leading bucket prefixes (e.g. `design-docs/`, `researches/`). It treats `<task-folder>/` as the task folder exactly as supplied (created lazily on the first consumer write). Each consuming skill MUST canonicalize `$ARGUMENTS` against its own convention BEFORE running Step 0:

| Consumer | Canonical task-folder path form | Canonicalization steps |
|:--|:--|:--|
| The `cafleet-design-doc` skill (create/execute/interview workflows) | `design-docs/<slug>` | (1) strip trailing `/design-doc.md` if present; (2) strip leading `design-docs/` if present; (3) prepend `design-docs/`. **Absolute paths**: apply ONLY step 1 (strip a trailing `/design-doc.md`); skip the bucket strip/prepend (steps 2–3) — the absolute path is used verbatim as the task folder (the trailing-filename strip stops a child file path from becoming a directory named after the file). |
| The `cafleet-research` skill (report/presentation workflows) | `researches/<topic-slug>` | (1) strip trailing `/report.md` (or other known per-topic filenames) if present; (2) strip leading `researches/` if present; (3) prepend `researches/`. **Absolute paths**: apply ONLY step 1 (strip a trailing `/report.md`); skip the bucket strip/prepend (steps 2–3) — the absolute path is used verbatim as the task folder. |

Skipping canonicalization resolves the wrong BASE — a directory literally named `design-doc.md` (or `report.md`, etc.) instead of the intended task folder.

### Step 1. Shared-root resolution (no task name)

When the consuming skill has no per-task folder convention (the shared-root case):

1. Determine the CWD and `$HOME`. The user-level config directories are checked for **all** backends regardless of which one is resolving — resolution needs no backend identity, and a Director of one backend whose CWD sits inside another backend's config dir is still in an operator-config location, not a project:

   | Backend | User-level config dir |
   |:--|:--|
   | claude | `~/.claude` |
   | codex | `~/.codex` |
   | opencode | `~/.config/opencode` |

2. **If the CWD is neither `$HOME` itself nor a config dir from the table nor any directory under one** → `${BASE}` is the CWD itself (the working directory, **not** the repo root). Done. (Only `$HOME` exactly and the config-dir subtrees fall through to Step 2 — an ordinary project elsewhere under `$HOME` resolves here.)
3. **Otherwise** (CWD is `$HOME` exactly, or a config dir from the table / any directory under one) → go to Step 2 with candidates `[/tmp/cafleet, <CWD>]`.

### Step 2. Decision-surface prompt (only when Step 1 reaches branch 3)

Present the candidates via `{decision_surface}` ("Select the base directory for output files:"):

- `/tmp/cafleet (recommended)` → `${BASE} = /tmp/cafleet`
- `${CWD}` → `${BASE} = ${CWD}` (the CWD literal from the candidates list)
- `Other` (free text) → `${BASE} = user input` (resolve against `${CWD}` if relative)

The chosen value is `${BASE}`. Nothing is persisted; if the procedure reloads in the same `$HOME` / config-dir CWD, this branch re-prompts.

## No-bypass write protocol

Every CAFleet member, every consumer skill, and every Director MUST follow this protocol for scratch / audit / figure / spawn-prompt-render writes:

1. **Every write under `${BASE}` or an explicit consumer-supplied absolute target.** Scratch (pre-spawn renders of spawn prompts at `${BASE}/.prompts/<role>-<UTC-compact>.md`, working notes), audit files, figure artifacts, and any other ephemeral output MUST land under `${BASE}` or under a consumer-supplied absolute path — e.g., the design-doc directory delivered to spawned members via `[INSERT abs design-doc directory]`, or the research folder delivered via `[INSERT abs research folder]`. Never `/tmp` unless `${BASE}` itself is `/tmp/cafleet` (which is a legitimate base-dir choice).

2. **Spawn prompts are written before they are spawned (the two-step).** Render each spawn prompt and **write** it to `${BASE}/.prompts/<role>-<UTC-compact>.md`, then invoke `cafleet member create --text-file <abs path>` against that file — the pre-spawn file is both the CLI input and the permanent audit artifact. Passing the prompt by path is required, not stylistic: `member create` hands the prompt to `tmux split-window` as one positional argument, so an inline prompt fails with `command too long` past a few KB. The CLI runs `str.format` over the file, rendering the four identity placeholders to literals at spawn, so double any literal brace as `{{` / `}}` and leave no other stray single braces. The `<UTC-compact>` format, the same-second collision rule, the identity-placeholders-pre-substitution note, and the inline fallback when `${BASE}` is `<unset>` are canonical in [`reference/director.md`](director.md) § *Member Create — Scratch and audit files* and § *Spawn prompt size limit*.

3. **`${BASE} == <unset>` is a hard stop, not a fallback.** If `${BASE}` is the literal sentinel `<unset>` (absolute-path argument branch), any code that tries to compute a path from `${BASE}` MUST abort with `Error: BASE is <unset>; refusing to fall back to /tmp`. The loud failure is the safety net for sites that forgot to guard explicitly.

4. **Members never re-resolve BASE.** The Director's spawn-prompt substitution delivers `${BASE}` to each spawned member as a literal absolute path baked into the spawn prompt. Members MUST use that literal path verbatim. Members DO Read this file at startup (per their role file's *Required reading* block) to pick up the no-bypass write protocol and the `<unset>` sentinel contract — but they MUST NOT run the resolution procedure (Steps 0–2) or otherwise derive a new `${BASE}` of their own. Re-resolving would invite drift if the Director's resolved BASE changed mid-session.

5. **Missing-BASE-line anchorless status.** If a member's spawn prompt is missing the `BASE:` line entirely (an expected outcome when the Director resolved `${BASE} = <unset>`), the member treats the audit-file feature as disabled and emits a single CAFleet message back to the Director as a parens-free anchorless status (per [`../../cafleet-design-doc/reference/coordination.md`](../../cafleet-design-doc/reference/coordination.md) § *Anchorless Status*):

   ```
   audit-disabled no BASE in spawn prompt
   ```

   The phrasing deliberately omits parentheses so a Director reading the broker log does not misinterpret it as a malformed `<verb> (<pointer>)` hop. The member MUST NOT fall back to `/tmp`.

### Hidden agent-only folders vs visible deliverables

Assets a coding agent creates only for its own workflow — scratch, audit trails, and intermediate build inputs — live in **dot-prefixed hidden folders** under `${BASE}` (e.g. `${BASE}/.prompts/`, `${BASE}/.figures/code`, `${BASE}/.figures/data`, `${BASE}/.screenshots/`). Assets that are **user-facing deliverables** — the artifacts the user opens, embeds, or ships — live in **visible, unprefixed folders** (e.g. `${BASE}/figures/output/` for the rendered charts embedded into slides and reports).

When a skill adds a new output folder under `${BASE}`, classify it first: coding-agent-only → dot-prefix it; user-facing deliverable → leave it visible/unprefixed.

## The `<unset>` sentinel

This resolution procedure is the only producer of `BASE=<unset>` — the literal string `"<unset>"` (case-sensitive), returned by the absolute-path-arg branch (Step 0). A consumer with `${BASE} == <unset>` follows the **No-bypass write protocol** above: **guard** every audit-file write with an explicit `${BASE} != <unset>` check at the call site (a guarded skip is the intended path — no silent no-op, no `/tmp` fallback); **omit** the `BASE:` line entirely from spawn prompts (never write the literal `BASE: <unset>`), so the member's existence-check treats audit-files as disabled; and **loud-error** on any unguarded `Path(BASE) / …` computation (the abort string in item 3).
