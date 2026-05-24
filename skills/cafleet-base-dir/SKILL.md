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

The deterministic side of resolution is implemented in Python and exposed via `cafleet base-dir resolve` / `cafleet base-dir record` (see § *CLI surface* below). Claude's job is to (a) invoke `cafleet base-dir resolve` — with a positional `TASK_NAME` when the consuming skill operates on a per-task folder, otherwise without — (b) branch on the `status` field, (c) drive `AskUserQuestion` only when `status == "needs-user-input"`, and (d) call `cafleet base-dir record` to persist the answer.

### Step 0. Task-scope resolution (preferred for task-aware consuming skills)

When the consuming skill operates on a per-task folder, it picks the task-relpath itself (the resolver is general-purpose and does NOT enumerate or special-case any bucket name — `researches/`, `design-docs/`, etc. are all consumer-side conventions). Pass the relpath as a positional argument and skip Steps 1–2:

```bash
cafleet base-dir resolve <task-relpath> --json
# e.g. cafleet base-dir resolve researches/my-topic --json
# e.g. cafleet base-dir resolve design-docs/0000060-skill-task-scoped-base-dir --json
```

The CLI:

1. Walks up from CWD via `is_git_repo_root` to infer the repo root.
2. Joins `<task-relpath>` against the repo root and resolves the path. Rejects traversal escapes (`../outside`, `design-docs/../../escaped`) and the repo-root degenerate (`.`, `""`, `design-docs/..`) with `RuntimeError`.
3. Auto-creates the task folder via `pathlib.Path(...).mkdir(parents=True, exist_ok=True)` — no shell-out.
4. Writes a per-task anchor inline at `<task-folder>/.cafleet-base-dir.json` with `source: "task-scope"` (or reads an existing one with `source: "anchor"`).
5. Returns `{status: "resolved", base: <abs task-folder>, source: "task-scope" | "anchor", anchor: <abs anchor>, task_name: <task-relpath>}`.

`${BASE}` is the task folder itself, so `${BASE}/prompts/<role>-<UTC-compact>.md` audit files land under the task folder rather than at the repo root. **The consuming skill is responsible for passing a relpath that is the actual task folder**, not a child file path — the resolver does no slug-folding or filename-stripping.

If the consuming skill received an absolute path (e.g. `/abs/path/to/task-folder`), pass it through positionally:

```bash
cafleet base-dir resolve <abs-path> --json
```

The CLI checks whether the absolute path is strictly under the inferred repo root. If so, the path is treated as the task folder verbatim (auto-created, anchored, returned as `task-scope`). If the path lies outside the repo root or equals the repo root (which would clobber the shared-root anchor), the CLI returns the `unset` shape (`status: "unset"`, `source: "absolute-path-arg"`). No bucket-pattern ancestor walk; no skill-specific slug matching.

**Consumer contract — canonicalize ARGUMENTS to the task-folder relpath before calling.** The resolver is deliberately general: it does not strip trailing filenames (e.g. `/design-doc.md`, `/report.md`) and does not strip leading bucket prefixes (e.g. `design-docs/`, `researches/`). It creates `<task-folder>/` exactly as supplied. Each consuming skill MUST canonicalize `$ARGUMENTS` against its own convention BEFORE calling `cafleet base-dir resolve`:

| Consumer | Canonical relpath form | Canonicalization steps |
|:--|:--|:--|
| The `cafleet-design-doc-create` / `cafleet-design-doc-execute` / `cafleet-design-doc-interview` skills | `design-docs/<slug>` | (1) strip trailing `/design-doc.md` if present; (2) strip leading `design-docs/` if present; (3) prepend `design-docs/`. **Absolute paths**: apply the same `/design-doc.md` strip (the resolver does not fold child filenames; a child file path becomes a directory named after the file). |
| The `cafleet-research-report` / `cafleet-research-presentation` skills | `researches/<topic-slug>` | (1) strip trailing `/report.md` (or other known per-topic filenames) if present; (2) strip leading `researches/` if present; (3) prepend `researches/`. **Absolute paths**: apply the same `/report.md` strip (the resolver does not fold child filenames; a child file path becomes a directory named after the file). |

The stripping logic is skill-specific because each skill knows the file conventions inside its own bucket. The resolver stays bucket-agnostic. Skipping canonicalization causes the resolver to create a directory literally named `design-doc.md` (or `report.md`, etc.) and anchor the wrong BASE.

When `_infer_repo_root(cwd)` returns `None` (CWD has no `.git` ancestor — typical when CWD is `$HOME` or under `$HOME/.claude`) AND a positional `TASK_NAME` is supplied, the CLI exits 1 with `cannot resolve task-scope base-dir: no .git ancestor found from CWD <cwd>. cd to the repo root and retry.` on stderr (no JSON payload, even with `--json`). `cd` to the repo root and retry.

### Step 1. Probe via the CLI (no-positional / shared-root case)

When the consuming skill has no task-folder convention (the shared-root case), run the no-positional form:

```bash
cafleet base-dir resolve --json
```

The CLI returns one of three JSON shapes (plus a fourth fatal branch that exits non-zero with an error message rather than a JSON payload):

| `status` field | Meaning | `${BASE}` outcome |
|:--|:--|:--|
| `resolved` | A concrete BASE was determined from CWD inference or from an existing anchor. The `base` field is the absolute path; the `source` field is `cwd-inference` or `anchor`. | `${BASE} = <base>`. Done. |
| `needs-user-input` | The CLI could not resolve deterministically (CWD is `$HOME` or under `$HOME/.claude`, and no usable anchor exists). The `candidates` field lists the options to present. | Go to Step 2. |
| (no JSON — non-zero exit) | Anchor schema mismatch, version mismatch, or other fatal condition. The CLI exits non-zero with an error message on stderr; no JSON payload is emitted. | Surface the error and stop — do NOT fall back to `/tmp`. |

The `unset` status is emitted only by the positional branch (Step 0) when an absolute-path `TASK_NAME` lies outside the inferred repo root (or equals the repo root itself); the no-positional probe never produces it.

### Step 2. AskUserQuestion (only when `status == "needs-user-input"`)

Present the candidates via `AskUserQuestion` ("Select the base directory for output files:"):

- `/tmp/claude-code (recommended)` → `${BASE} = /tmp/claude-code`
- `${CWD}` → `${BASE} = ${CWD}` (the CWD literal from the `candidates` array)
- `Other` (free text) → `${BASE} = user input` (resolve against `${CWD}` if relative)

Then persist the answer:

```bash
cafleet base-dir record --base <abs-path> --source askuserquestion
```

`record` is idempotent: re-running with the same `--base` is a no-op; re-running with a mismatched `--base` against an existing anchor errors and exits non-zero (the existing anchor wins; the caller must delete it manually if intentional).

### Anchor file

`cafleet base-dir record` writes an anchor file alongside the resolved BASE so that re-loading this skill (after auto-compression, after a fresh Director, after the next session) returns the same answer without re-prompting. The positional task-scope branch ALSO writes an anchor — inline, inside the task folder — so each task carries its own independent anchor under the same schema.

- **Path**: `${BASE}/.cafleet-base-dir.json`. For the no-positional branch this is the repo root or `/tmp/claude-code`; for the positional task-scope branch this is `<task-folder>/.cafleet-base-dir.json` (one anchor per task).
- **Schema** (version 1):

  ```json
  {
    "version": 1,
    "base": "/abs/path/to/base",
    "source": "task-scope",
    "resolved_at": "2026-05-12T13:00:00.000000+00:00"
  }
  ```

  - `version`: `1`. Future versions land alongside an explicit upgrade path. `resolve` rejects any anchor whose `version` field is missing, not a positive integer, or not `1` with a single standardized error message — silent forward-compatibility would let two installations on the same machine at different cafleet versions disagree about BASE.
  - `base`: absolute path. MUST equal the directory the anchor lives in. Mismatch is a fatal error in `resolve`.
  - `source`: one of `"cwd-inference"` (anchor written automatically when CWD inference produced a BASE), `"askuserquestion"` (anchor written after the user picked an option via `cafleet base-dir record`), or `"task-scope"` (anchor written inline by the positional task-scope branch on first resolution of a task folder).
  - `resolved_at`: ISO 8601, UTC, microsecond precision, `+00:00` suffix (same convention as cafleet `status_timestamp`).

The anchor at `/tmp/claude-code/.cafleet-base-dir.json` does NOT survive reboot — acceptable. After reboot the `AskUserQuestion` branch fires once and re-creates the anchor. Per-task anchors at `<task-folder>/.cafleet-base-dir.json` survive across sessions and are picked up by subsequent `cafleet base-dir resolve <task-name>` invocations as `source: "anchor"`.

### Gitignore handling

When `${BASE}` is a git-repo CWD (the normal case in this project), the anchor file would otherwise show up in `git status`.

1. **Host projects should exclude `.cafleet-base-dir.json` (unanchored — matches both the repo-root anchor AND per-task anchors under `<task-folder>/.cafleet-base-dir.json` written by the positional task-scope branch)** so anchor files do not surface as untracked. This project's `.gitignore` uses the unanchored pattern.
2. **`cafleet base-dir record` does NOT mutate any `.gitignore`** — silently editing a user repo's gitignore as a side-effect would be a surprising write.
3. **Users who do not want anchor noise in arbitrary repos should add `.cafleet-base-dir.json` to their global excludes** (`~/.config/git/ignore` or `core.excludesFile`). The first `record` call under a fresh git-repo BASE prints a one-line stderr tip pointing at this recommendation.

## CLI surface

The two subcommands live under the `cafleet` CLI as the `base-dir` subgroup. Like `cafleet doctor`, they operate on the local filesystem only and do NOT require `--session-id`.

```bash
# Non-interactive resolution. Never prompts the user.
#
# No-positional form: walks CWD inference / anchor / askuserquestion. On the
# CWD-inference branch it auto-writes the anchor at ${CWD}/.cafleet-base-dir.json
# so the next call returns from "anchor" instead.
#
# Positional form: TASK_NAME is a relative path under the inferred repo root
# (e.g. researches/<slug>, design-docs/<NNNNNNN>-<slug>) or an absolute path
# strictly under the repo root. The CLI auto-creates the task folder (verbatim
# — no skill-specific ancestor walk), writes <task-folder>/.cafleet-base-dir.json
# with source: "task-scope", and returns the task folder as the resolved base.
cafleet base-dir resolve [TASK_NAME] [--json]

# Possible JSON outputs:
# {"status": "resolved", "base": "/home/user/proj", "source": "cwd-inference", "anchor": "/home/user/proj/.cafleet-base-dir.json"}
# {"status": "resolved", "base": "/tmp/claude-code", "source": "anchor", "anchor": "/tmp/claude-code/.cafleet-base-dir.json"}
# {"status": "resolved", "base": "/home/user/proj/researches/my-topic", "source": "task-scope", "anchor": "/home/user/proj/researches/my-topic/.cafleet-base-dir.json", "task_name": "researches/my-topic"}
# {"status": "unset", "base": null, "source": "absolute-path-arg", "anchor": null, "task_name": "/abs/path/outside/any/task/folder"}
# {"status": "needs-user-input", "base": null, "source": null, "candidates": ["/tmp/claude-code", "<cwd>"]}
#
# Fatal positional-branch failure (no .git ancestor): exit 1 with the message
# "cannot resolve task-scope base-dir: no .git ancestor found from CWD <cwd>.
# cd to the repo root and retry." on stderr — no JSON payload, even with --json.

# Persist an anchor after AskUserQuestion. Claude calls this after presenting the picker.
# Note: the positional task-scope branch above writes its own anchor inline, so
# `record` is reserved for the explicit-AskUserQuestion flow that targets the
# shared root. `--source` accepts {askuserquestion, cwd-inference} only —
# "task-scope" is not a valid `record` source.
cafleet base-dir record --base <abs-path> --source askuserquestion
# Writes <base>/.cafleet-base-dir.json with version=1 and a UTC resolved_at.
# Idempotent: if the anchor already exists and matches, no-op; if it mismatches, error.
```

Default text output is human-readable; `--json` switches to machine-parseable.

## No-bypass write protocol

Every CAFleet member, every consumer skill, and every Director MUST follow this protocol for scratch / audit / figure / spawn-prompt-render writes:

1. **Every write under `${BASE}` or an explicit consumer-supplied absolute target.** Scratch (pre-spawn renders of spawn prompts at `${BASE}/prompts/<role>-<UTC-compact>.md`, working notes), audit files, figure artifacts, and any other ephemeral output MUST land under `${BASE}` or under a consumer-supplied absolute path — e.g., the design-doc directory delivered to spawned members via `[INSERT abs design-doc directory]`, or the research folder delivered via `[INSERT abs research folder]`. Never `/tmp` unless `${BASE}` itself is `/tmp/claude-code` (which is a legitimate base-dir choice).

2. **`${BASE} == <unset>` is a hard stop, not a fallback.** If `${BASE}` is the literal sentinel `<unset>` (absolute-path argument branch), any code that tries to compute a path from `${BASE}` MUST abort with `Error: BASE is <unset>; refusing to fall back to /tmp`. The loud failure is the safety net for sites that forgot to guard explicitly.

3. **Members never re-resolve BASE.** The Director's spawn-prompt substitution delivers `${BASE}` to each spawned member as a literal absolute path baked into the spawn prompt. Members MUST use that literal path verbatim. Members DO load this skill at startup (per their role file's *Load at Startup* block) to pick up the no-bypass write protocol and the `<unset>` sentinel contract — but they MUST NOT run `cafleet base-dir resolve` / `cafleet base-dir record` or otherwise derive a new `${BASE}` of their own. Re-resolving would invite drift if the Director's anchor changed mid-session.

4. **Missing-BASE-line anchorless status.** If a member's spawn prompt is missing the `BASE:` line entirely (an expected outcome when the Director resolved `${BASE} = <unset>`), the member treats the audit-file feature as disabled and emits a single CAFleet message back to the Director as a parens-free anchorless status (per `skills/design-doc/coordination.md` § *Anchorless Status*):

   ```
   audit-disabled no BASE in spawn prompt
   ```

   The phrasing deliberately omits parentheses so a Director reading the broker log does not misinterpret it as a malformed `<verb> (<pointer>)` hop. The member MUST NOT fall back to `/tmp`.

## The `<unset>` sentinel

The `cafleet-base-dir` skill is the only place that may return `BASE=<unset>`. The sentinel is the literal string `"<unset>"` (case-sensitive). Any consumer skill that has `${BASE}` set to `<unset>` MUST:

1. **Guard audit-file writes.** Audit-file writes that derive their path from `${BASE}` are guarded by an explicit `${BASE} != <unset>` check at the call site. The guard is mandatory; reaching `Path(BASE) / …` without the guard is the failure mode item 3 catches. A guarded skip is the intended path when `${BASE}` is `<unset>` — no silent no-op, no fallback to `/tmp`, just an explicit skip whose condition is visible in the source.
2. **Omit `BASE:` from spawn prompts.** The Director MUST NOT spawn members with `BASE: <unset>` in the spawn prompt — the line is omitted entirely so the member's existence-check naturally treats audit-file features as disabled. The literal string `BASE: <unset>` is never written into any spawn prompt.
3. **Loud-error on unguarded BASE-derivation.** If a code path under `${BASE}=<unset>` reaches an unguarded `Path(BASE) / …` computation, abort with the standardized error: `Error: BASE is <unset>; refusing to fall back to /tmp`.

Skip (item 1) and loud-error (item 3) are different stages of the same protocol, not alternative resolutions: every BASE-dependent write site MUST be guarded; the loud error is the safety net for sites that forgot to guard. The historical silent-skip behavior masked bypass bugs and is no longer accepted.
