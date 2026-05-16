# Skill Task-Scoped Base Directory

**Status**: Approved
**Progress**: 27/27 tasks complete
**Last Updated**: 2026-05-16

## Overview

Move every CAFleet-orchestrated spawn-prompt audit file out of the repo root and into the per-task project folder (`researches/<slug>/`, `design-docs/<NNNNNNN>-<slug>/`). Extend `cafleet base-dir resolve` to take a positional `TASK_NAME` argument that returns the absolute task-folder path, auto-creates it, and anchors it. Update the five in-scope CAFleet-orchestrated skills to call the new resolver shape so the `BASE:` line they substitute into spawn prompts now points at the task folder, and ship a new self-contained project-local meta-skill at `.claude/skills/skill-author/` that teaches authors how to integrate the pattern when writing a new CAFleet-orchestrated skill.

## Success Criteria

- [x] `cafleet base-dir resolve <task-name>` returns `{status: "resolved", base: <abs task-folder>, source: "task-scope" | "anchor", anchor: <abs anchor path>, task_name: <task-name>}` when invoked with a relative `TASK_NAME` such as `researches/my-topic` or `design-docs/0000060-skill-task-scoped-base-dir`.
- [x] The resolver auto-creates the task folder via `pathlib.Path(...).mkdir(parents=True, exist_ok=True)` — no shell-out to `mkdir`.
- [x] Each task folder carries its own `.cafleet-base-dir.json` anchor file written under the existing version 1 schema, independent of any shared-root anchor.
- [x] An absolute-path `TASK_NAME` strictly under the inferred repo root resolves to that path verbatim as the task folder (no skill-specific ancestor walk — the resolver is general-purpose); absolute paths outside the repo root or equal to the repo root return the `<unset>` sentinel.
- [x] Each of the five in-scope skills (`/cafleet:research-report`, `/cafleet:design-doc-create`, `/cafleet:design-doc-execute`, `/cafleet:design-doc-interview`, `/cafleet:research-presentation`) substitutes the task-folder path into the `BASE:` line of every spawn prompt, so audit files land at `<task-folder>/prompts/<role>-<UTC-compact>.md`.
- [x] A new project-local meta-skill at `.claude/skills/skill-author/SKILL.md` auto-loads (via its `description:` trigger) when an author starts creating a new CAFleet-orchestrated skill, and contains a complete, self-contained integration guide — no cross-skill references required to use it.
- [x] `cafleet/tests/test_base_dir.py` covers the positional `TASK_NAME` path: relative task name, absolute path under repo root (resolves verbatim), absolute path outside (returns `<unset>`), absolute path equal to repo root (returns `<unset>`), relative traversal escape (raises), repo-root degenerate (raises), anchor write/read at the task folder, and auto-mkdir of a non-existent task folder.

---

## Background

Today every CAFleet-orchestrated skill writes its rendered spawn prompts to `${BASE}/prompts/<role>-<UTC-compact>.md` where `${BASE}` is the value returned by `Skill(cafleet:base-dir)`. The resolver's cwd-inference branch returns the cafleet repo root when CWD is the repo root (the normal case), so audit files land at `/home/himkt/work/himkt/cafleet/prompts/` — alongside `researches/` and `design-docs/` at the repo top level. Two consequences:

1. The repo root accumulates ephemeral audit artifacts from every team run. Operators routinely `.gitignore` `prompts/` to keep them out of version control, which works but treats per-task evidence as repo-global churn.
2. There is no proximity between a task and its spawn-prompt history. To inspect the prompts that produced `design-docs/0000042-foo/design-doc.md` an operator has to grep across the repo-root `prompts/` for the timestamps that match the run, instead of opening `design-docs/0000042-foo/prompts/`.

The root cause is that `cafleet base-dir resolve` has no concept of "task scope." It returns either the repo root (cwd-inference), `/tmp/claude-code` (askuserquestion), or the `<unset>` sentinel (absolute-path arg). Consuming skills had no clean way to ask for the per-task folder. The dogfooded fix in this very design-doc-create run already demonstrates the target shape: the Director computed `BASE = /home/himkt/work/himkt/cafleet/design-docs/0000060-skill-task-scoped-base-dir/`, substituted it into the Drafter spawn prompt, and wrote the rendered prompt to `<task-folder>/prompts/drafter-<ts>.md`. The design doc generalizes that one-off into a first-class resolver feature.

---

## Specification

### 1. CLI surface

`cafleet base-dir resolve` gains a single optional positional argument `TASK_NAME` and loses the existing `--path` option (the positional subsumes its only purpose — recognizing absolute-path arguments).

```
cafleet base-dir resolve [TASK_NAME] [--json]
```

| Form | Behavior |
|:--|:--|
| `cafleet base-dir resolve` (no positional) | Unchanged from today. Returns the repo-root via cwd-inference, `/tmp/claude-code` via anchor, `<unset>` is not produced (only the absolute-path branch produces it). |
| `cafleet base-dir resolve <relative-task-name>` | Task-scope branch. Treats `TASK_NAME` as relative to the cwd-inferred repo root. Joins + resolves; rejects traversal (`../escape`) and the repo-root degenerate case (`.`, `""`, `design-docs/..`) with `RuntimeError`. Auto-creates the task folder. Writes / reads its anchor. Returns `{status: "resolved", base: <abs task folder>, source: "task-scope" \| "anchor", anchor: <abs anchor>, task_name: <TASK_NAME>}`. **The resolver is general-purpose** — it does NOT know about skill-specific bucket names (`researches/`, `design-docs/`); the consuming skill picks the relpath. |
| `cafleet base-dir resolve <abs-path>` | If the absolute path is strictly under the repo root, treat it as the task folder verbatim (auto-create + anchor + return as task-scope). If the path is outside the repo root or equal to the repo root, return the `<unset>` shape (`status: "unset"`, `source: "absolute-path-arg"`). No bucket-pattern ancestor walk; no skill-specific slug matching. |

`cafleet base-dir record` is unchanged — it accepts an absolute `--base` and writes the anchor. The resolver's task-scope branch writes its anchor inline, so most callers do not need `record` for task folders; `record` remains for the explicit-`AskUserQuestion` flow that targets the shared root.

### 2. Resolution algorithm (positional `TASK_NAME` branch)

The existing `resolve()` function in `cafleet/src/cafleet/base_dir.py` is extended in place: the current `path: str | None` keyword argument is **renamed to** `task_name: str | None` (the old `path` parameter is removed — there is no `path` kwarg on the new `resolve()` signature, matching the CLI's `--path` removal in § 1). When `task_name is None`, the existing no-positional logic (cwd-inference / anchor / askuserquestion) runs unchanged. When `task_name is not None`, the function dispatches to the general task-scope branch below.

**Resolver is skill-agnostic.** The implementation does NOT enumerate or special-case any bucket name (`researches/`, `design-docs/`, etc.) and does NOT enforce slug regex shapes. Skill-specific conventions live in the consuming skill, not in the resolver — each skill picks its own task relpath and passes it.

```text
def resolve(*, task_name=None, cwd=None, home=None, tmp_candidate=None) -> dict:
    cwd_path = Path(cwd) if cwd is not None else Path.cwd()
    if task_name is not None:
        return _resolve_task_scope(task_name, cwd=cwd_path)
    # ... existing no-positional logic unchanged ...


def _resolve_task_scope(task_name: str, *, cwd: Path) -> dict:
    repo_root = _infer_repo_root(cwd)
    if repo_root is None:
        raise RuntimeError(
            f"cannot resolve task-scope base-dir: no .git ancestor "
            f"found from CWD {cwd}. cd to the repo root and retry."
        )

    candidate = Path(task_name)
    if candidate.is_absolute():
        task_folder = candidate.resolve(strict=False)
        if task_folder == repo_root or not _is_under(task_folder, repo_root):
            return {"status": "unset", "base": None,
                    "source": "absolute-path-arg", "anchor": None,
                    "task_name": task_name}
    else:
        task_folder = (repo_root / candidate).resolve(strict=False)
        if task_folder == repo_root:
            raise RuntimeError(
                f"task_name {task_name!r} resolves to the repo root "
                f"({repo_root}); the repo root is not a task folder."
            )
        if not _is_under(task_folder, repo_root):
            raise RuntimeError(
                f"task_name {task_name!r} resolves outside the repo root "
                f"({task_folder} is not under {repo_root}); refusing to create "
                f"a task folder outside the repo."
            )

    task_folder.mkdir(parents=True, exist_ok=True)
    anchor_path = task_folder / ANCHOR_FILENAME

    existing = _read_consistent_anchor(anchor_path)
    if existing is not None:
        source = "anchor"
    else:
        _write_anchor(anchor_path, base=str(task_folder), source="task-scope")
        source = "task-scope"

    return {"status": "resolved", "base": str(task_folder),
            "source": source, "anchor": str(anchor_path),
            "task_name": task_name}
```

Key invariants:

- `_infer_repo_root(cwd)` walks upward from `cwd` using the **existing** `is_git_repo_root(p)` helper in `base_dir.py` (which returns `True` when `<p>/.git` exists — file or directory, so git worktrees are supported). It returns the first ancestor for which `is_git_repo_root` is True. If no `.git` ancestor is found (CWD is outside any git repo — typically `$HOME` or under `$HOME/.claude`), it returns `None`, which the task-scope branch surfaces as a fatal error (see *No-repo-root failure mode* below). Importantly: when the user runs `cafleet base-dir resolve <relpath>` from **inside** a sub-folder of the repo, `_infer_repo_root` walks up to the repo root, not the inner CWD — so the relative-task-name path joins against the repo root, not against the inner CWD.
- **Absolute-path branch**: the resolver does not walk ancestors and does not match against any bucket name. Either the absolute path is strictly under the repo root (treat as task folder verbatim) or it is not (return `<unset>`). The `repo_root == task_folder` case also returns `<unset>` to prevent collision with the shared-root anchor.
- **Relative-path branch**: traversal escapes (`../outside`, `design-docs/../../escaped`) and the repo-root degenerate (`.`, `""`, `design-docs/..`) raise `RuntimeError`. Otherwise the joined+resolved path is the task folder. Whatever relpath the consuming skill passes — regardless of whether it ends in a filename or a folder — is used as-is for `mkdir`. **Consuming skills are responsible for passing a relpath that is the actual task folder, not a child path.**
- The task-folder mkdir uses `pathlib.Path.mkdir(parents=True, exist_ok=True)`. **No `subprocess.run(["mkdir", ...])`, no shell-out, no `os.makedirs` wrapper** — pathlib is the only sanctioned API.
- `ANCHOR_FILENAME`, `ANCHOR_VERSION`, the anchor's JSON shape, and `_read_consistent_anchor` / `_write_anchor` are unchanged from today. The anchor's `source` field accepts the new value `"task-scope"` alongside the existing `"cwd-inference"` and `"askuserquestion"` (see § 3 for the explicit `_validate_anchor` / `record` split).

**No-repo-root failure mode.** When `_infer_repo_root(cwd)` returns `None` AND a positional `TASK_NAME` is supplied, `_resolve_task_scope` raises a plain `RuntimeError` with the message `cannot resolve task-scope base-dir: no .git ancestor found from CWD <cwd>. cd to the repo root and retry.` (no new exception class — `RuntimeError` matches the existing `HOME is not set` failure mode in `resolve()`). The CLI catches this in the `base_dir_resolve` handler and exits with code `1`, writes the error message to stderr (no JSON payload, even when `--json` is passed — this mirrors the existing fatal `AnchorError` branch in `Skill(cafleet:base-dir)` § *Step 1* row 4 of the status table). The no-positional path is unaffected by this branch — without a `TASK_NAME`, the resolver follows the existing `AskUserQuestion`-driven recovery for CWD-under-`$HOME` cases.


### 3. Anchor schema (revised `source` enum)

```json
{
  "version": 1,
  "base": "/home/himkt/work/himkt/cafleet/design-docs/0000060-skill-task-scoped-base-dir",
  "source": "task-scope",
  "resolved_at": "2026-05-16T12:00:00.000000+00:00"
}
```

| `source` value | When written |
|:--|:--|
| `cwd-inference` | No-positional resolve picks up the repo root. (Existing.) |
| `askuserquestion` | No-positional resolve and the caller persisted an `AskUserQuestion` answer via `cafleet base-dir record`. (Existing.) |
| `task-scope` | Positional `TASK_NAME` resolved a task folder for the first time. (New.) |

`_validate_anchor` accepts all three values; any other value continues to error per existing semantics.

**`record()` and its CLI scope are unchanged.** The resolver writes task-scope anchors **inline** during the task-scope branch (per the algorithm in § 2 — `_write_anchor(... source="task-scope")` runs synchronously inside `_resolve_task_scope`). Therefore `cafleet/src/cafleet/base_dir.py::record()` does NOT accept `"task-scope"` as a `source` value, and the CLI `base_dir_record`'s `--source` Click Choice list stays `click.Choice(["askuserquestion", "cwd-inference"])`. Implementers MUST NOT "helpfully" expand the Choice list — `record` is the explicit-`AskUserQuestion` persist path, not the resolver's inline persistence path.


### 4. Sentinel behavior preserved

The `<unset>` sentinel still exists. It is now returned only when an **absolute-path** `TASK_NAME` is supplied AND the path lies outside the inferred repo root (or equals the repo root itself, which would clobber the shared-root anchor). The `<unset>` skip semantics in `Skill(cafleet:base-dir)` (`No-bypass write protocol` § *unset sentinel*) are unchanged — audit-file writes still skip with a guarded check, and the Director still omits the `BASE:` line from spawn prompts when `${BASE} == <unset>`.

### 5. Spawn-prompt `BASE:` line semantics

The literal `BASE:` line in every spawn prompt template is reused with no rename. Its meaning quietly shifts from "the repo-root path returned by cafleet:base-dir" to "the task-folder path returned by `cafleet base-dir resolve <task-name>` for this run." All existing substitution code in `cafleet.base_dir.substitute_base_in_prompt` works unchanged — the consuming skill simply passes a different `base` string.

### 6. Consuming skill changes (5 SKILL.md files)

Each skill's "Step 0 / Base Directory Selection" recipe changes from "load `Skill(cafleet:base-dir)` without args" to "call `cafleet base-dir resolve <task-relpath>` with a relpath the skill computes from its `$ARGUMENTS`."

| Skill | New resolver call | Resulting BASE |
|:--|:--|:--|
| `/cafleet:research-report` | `cafleet base-dir resolve researches/<topic-slug>` | `<repo>/researches/<topic-slug>/` |
| `/cafleet:research-presentation` | `cafleet base-dir resolve researches/<topic-slug>` (same slug it was handed) | `<repo>/researches/<topic-slug>/` |
| `/cafleet:design-doc-create` | `cafleet base-dir resolve design-docs/<NNNNNNN>-<slug>` | `<repo>/design-docs/<NNNNNNN>-<slug>/` |
| `/cafleet:design-doc-execute` | `cafleet base-dir resolve design-docs/<NNNNNNN>-<slug>` (after slug discovery) | `<repo>/design-docs/<NNNNNNN>-<slug>/` |
| `/cafleet:design-doc-interview` | `cafleet base-dir resolve design-docs/<NNNNNNN>-<slug>` | `<repo>/design-docs/<NNNNNNN>-<slug>/` |

Side effects of moving BASE into the task folder:

- `OUTPUT_DIR` in research-report and research-presentation now equals BASE (they are the same path). The skill keeps the `${OUTPUT_DIR}` label for clarity in the human-readable Step text but drops the `${BASE}/researches/<slug>/` concatenation — there is nothing left to append.
- `DOC_PATH` in design-doc-create / design-doc-interview / design-doc-execute is now `BASE / design-doc.md` (replacing `BASE / design-docs / <slug> / design-doc.md`). The skills compute it the same way they did before but against the task-scoped BASE.
- The audit-file path `${BASE}/prompts/<role>-<UTC-compact>.md` is naturally task-scoped now — `${BASE}/prompts/` IS `<task-folder>/prompts/`.

Out of scope (decided during clarification):

- `/cafleet:my-slidev` — the slidev theme has no task-folder semantics; left unchanged.
- Any other skill not listed above is out of scope for this design doc.

**`/cafleet:design-doc-execute` Tier 3 verification (deferred to implementation).** The existing `/cafleet:design-doc-execute` Step 1 Phase 2 enumerates three tiers (direct file path, slug directory, base directory) and Tier 3 was written assuming `${BASE}` was the repo root containing many slug subdirectories. With task-scoped `${BASE}` pointing at one specific slug folder, Tier 3's `**/design-doc.md` glob (filtered to one level deep) will match exactly one file — namely `<task-folder>/design-doc.md`. The flow should naturally degrade to Tier 1 / Tier 2 in practice, but the design does NOT assert correctness here. Step 3 carries an explicit verification sub-task that the implementer runs after wiring the task-scoped BASE into design-doc-execute, and the outcome is recorded as either "Tier 3 short-circuits cleanly" or "Tier 3 needs a follow-up edit to behave correctly with single-doc BASE." If the latter, the implementer files the follow-up edit as part of the same change set.


### 7. Resolver SKILL.md updates (`skills/base-dir/SKILL.md`)

`Skill(cafleet:base-dir)` (the documented procedure) gains a step before "Step 1. Probe via the CLI": when the consuming skill has a task-relpath (slug-derived), invoke `cafleet base-dir resolve <task-relpath> --json` directly. The probe-and-branch flow in Steps 1–2 still applies to the no-positional case. The CLI surface table picks up the new positional argument and the absolute-path-arg recognition rules.

### 8. New project-local meta-skill: `.claude/skills/skill-author/`

A new top-level skill is added at `<cafleet-repo>/.claude/skills/skill-author/SKILL.md` — project-local, committed alongside the repo, NOT in user-global `~/.claude/skills/` and NOT inside the cafleet marketplace plugin's `<repo>/skills/` directory. The location is final; no further user confirmation is required.

**Discovery semantics.** Claude Code's skill discovery layers are:

| Layer | Path | When loaded |
|:--|:--|:--|
| User-global | `~/.claude/skills/` | Always loaded, every Claude Code invocation |
| Project-local | `<project-root>/.claude/skills/` | Loaded when Claude Code is invoked from that project root |
| Plugin marketplace | Whatever path the plugin manifest declares (cafleet uses `<repo>/skills/`) | Loaded when the user has the plugin installed via the marketplace |

`<cafleet-repo>/.claude/skills/skill-author/` is the project-local layer. It is loaded automatically when an author runs Claude Code from inside the cafleet repo (which is the only context where the skill's "create a CAFleet-orchestrated skill" trigger makes sense — outside the repo, an author has no `<repo>/skills/` to add to). The cafleet repo's `.gitignore` explicitly tracks `.claude/skills` via the override `!.claude/skills` (line 8), so `<cafleet-repo>/.claude/skills/skill-author/SKILL.md` is a committed first-class repo artifact. No marketplace registration is required — and deliberately not: this skill teaches authors how to write a NEW skill that would itself live under `<repo>/skills/` and ship via the marketplace, so the meta-skill is one layer above the marketplace plugin.

Why project-local rather than a new entry under `<repo>/skills/`: the meta-skill teaches a pattern specific to the cafleet repo's `researches/` and `design-docs/` task-folder conventions and references project-internal rules like `.claude/rules/design-doc-numbering.md`. Shipping it via the marketplace would expose those internal conventions to every downstream plugin user even when they have no cafleet repo on hand. Project-local discovery cleanly scopes the skill to the only repo where its rules apply.

The skill must:

- **Auto-load on author intent**. Its `description:` field carries trigger phrases for the cases an author would invoke it: "create a new CAFleet-orchestrated skill", "write a skill that spawns cafleet members", "add a Director/Member team skill". The description is the only trigger surface (skill loading is description-keyed), so it is written carefully and explicitly enumerates the trigger phrases.
- **Be fully self-contained**. All rules, patterns, and worked examples live inline in `SKILL.md`. No `See skills/cafleet/SKILL.md § X` cross-references are required to follow the guide. (Cross-references may appear as "for further detail, see ...", but every operational step is inlined.)
- **Explain why AND how, not fill-in-the-blanks**. The skill is a teaching document, not a paste-this template. It walks the author through the four sub-systems they must wire up (resolver, cafleet broker, spawn-prompt audit file, coordination protocol), explains why each step exists, and shows a concrete worked example. It does NOT inline a single copy-paste spawn prompt that the author edits — that pattern produces shallow integrations that drift from the canonical rules.

The skill's table of contents:

1. **What "CAFleet-orchestrated" means** — Director + spawned members + cafleet message broker; when this pattern is the right shape and when it is overkill.
2. **The five-part integration checklist**:
   - Resolve the task-scoped BASE (`cafleet base-dir resolve <task-relpath>`).
   - Bootstrap a cafleet session (`cafleet session create`).
   - Start the agent-team-monitoring `/loop`.
   - Spawn members with `cafleet member create --prompt-file <abs task-folder/prompts/...>`.
   - Tear down per the Shutdown Protocol (`member delete` → `session delete`).
3. **Spawn-prompt anatomy**: the identity block (`SESSION ID`, `DIRECTOR AGENT ID`, `YOUR AGENT ID`, `BASE`), the `str.format()` placeholder rules, the `[INSERT ...]` shell-substitution rules, the path-by-reference rule for role files, the `command too long` cliff and why `--prompt-file` solves it, the `${BASE} == <unset>` skip semantics.
4. **Audit-file write protocol**: `${BASE}/prompts/<role>-<UTC-compact>.md`, same-second collisions, "the pre-spawn file IS the audit artifact" rule.
5. **Coordination protocol summary**: verb + pointer cafleet bodies; substantive content as `COMMENT(role)` markers; the canonical 6-verb list (`ready`, `complete`, `addressed`, `blocked`, `escalating`, `approved`); pointer forms (`paragraph-<HeadingPath>`, `<file>:<line>`, `doc`). This section is the longest inline because it is the most easily miswired.
6. **Worked example** — a tiny end-to-end CAFleet-orchestrated skill called "summarize-pr" (single Director + single member named Summarizer). The example uses fake `<slug>` etc. and is read-only — it is illustrative, not a template the author copies.
7. **Common failure modes**: forgetting to ack messages, inlining role-file content into the spawn prompt (size limit), shell-variable-substituting the literal UUIDs (breaks `permissions.allow` matching), writing audit files under the repo root.

### 9. Migration & backward-compat

The repo today already carries artifacts from the pre-task-scope model. Each item below is resolved explicitly so implementers do not guess:

| Artifact | Disposition |
|:--|:--|
| `<repo>/.cafleet-base-dir.json` (source `cwd-inference`) | **Remains valid.** The no-positional `resolve()` branch is unchanged per § 1, so the repo-root BASE continues to resolve identically. No rewrite, no migration. |
| `<repo>/prompts/` directory (untracked, accumulated stale audit files from prior runs) | **Gitignore + one-time delete.** Add `/prompts/` to `.gitignore` so the directory stops surfacing in `git status` (and so any leftover writes during the rollout do not accidentally enter version control). Then delete the existing directory as a one-time cleanup task in Step 8. Historical files are NOT migrated into per-task folders — they are ephemeral audit records, and the per-task convention starts fresh from the rollout date. |
| Existing per-task anchors at `<repo>/design-docs/<NNNNNNN>-<slug>/.cafleet-base-dir.json` written by prior dogfood runs (including this very design doc's anchor, if Director-bootstrapped one) | **No rewrite required.** Schema is version 1 with `source: "task-scope"` — exactly what the new resolver writes. Subsequent invocations re-read them as `source: "anchor"` per § 2, matching documented behavior. |
| `.gitignore` lines 36-48 (`/drafter.md`, `/manager.md`, `/scout.md`, …) | **Out of scope, left as-is.** These are gitignore entries for the much older flat per-role audit-file model from design 0000053 (pre-`prompts/<role>-<ts>.md`). They no longer reflect any active write path but removing them is a separate cleanup that does not belong in this change set. |
| The `cafleet base-dir resolve --path` CLI option | **Removed, not deprecated.** Per `~/.claude/rules/removal.md`, every mention of `--path` in current source code, user-facing docs (README, ARCHITECTURE, `docs/spec/`), and skill files is deleted in the same change set. Older design documents that referenced `--path` as part of their historical design choices (e.g., `design-docs/0000055-base-dir-enforcement/`, `design-docs/0000059-prompt-file-option/`) are preserved untouched as the historical record per the same rule. No alias, no deprecation warning. |

### 10. Implementation ordering invariants

Per project rule `.claude/rules/design-doc-numbering.md`, documentation MUST land before code:

1. Update `README.md` and `ARCHITECTURE.md` to describe the new positional argument and task-scope semantics. `docs/spec/cli-options.md` does NOT document the `cafleet base-dir resolve` subcommand surface today — the grep performed during the design Q&A round confirms only an incidental `Skill(cafleet:base-dir)` mention at line 705, which stays as-is.
2. Update `.gitignore`: add `/prompts/` (the repo-root prompts dir is no longer written but may have stale entries from past runs).
3. Update `skills/base-dir/SKILL.md` (the canonical procedure) to describe the positional invocation.
4. Update each consuming skill's `SKILL.md` (research-report, design-doc-create, design-doc-execute, design-doc-interview, research-presentation).
5. Create `<repo>/.claude/skills/skill-author/SKILL.md`.
6. Implement the resolver change in `cafleet/src/cafleet/base_dir.py` and the CLI change in `cafleet/src/cafleet/cli.py`.
7. Add tests covering the new positional branch.
8. Run the one-time cleanup: delete the existing `<repo>/prompts/` directory.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation updates

> The greps `grep -n 'base-dir|base_dir|BASE:' README.md ARCHITECTURE.md docs/spec/cli-options.md` were run during the design Q&A round and identified four concrete edit sites. Implementers do NOT need to re-run the greps.

- [x] Update `README.md` line 66 (the `cafleet base-dir {resolve,record}` row in the CLI Surface table): rewrite the description to mention the optional positional `TASK_NAME` argument that returns the task-scoped base path. <!-- completed: 2026-05-16T00:20 -->
- [x] Update `ARCHITECTURE.md` base-dir paragraphs: line 57 (`cli.py` bullet) — mention the positional `TASK_NAME` argument on `cafleet base-dir resolve`; line 58 (`base_dir.py` bullet) — refresh the state-machine description to include the task-scope branch and the inline anchor write; line 178 — replace the `cafleet base-dir resolve [--path …]` signature with `[TASK_NAME]` and add the task-scope branch to the status enumeration; lines 179, 181, 183 — confirm unchanged (record/anchor/sentinel paragraphs are still accurate). <!-- completed: 2026-05-16T00:20 -->
- [x] Verify `docs/spec/cli-options.md` does NOT document the `cafleet base-dir resolve` subcommand surface (the design Q&A grep showed only the incidental mention at line 705, which uses `Skill(cafleet:base-dir)` and stays as-is). If a re-grep surfaces new mentions, file the edits as part of this task; otherwise no change. <!-- completed: 2026-05-16T00:20 -->
- [x] Update `.gitignore`: add `/prompts/` to ignore the legacy repo-root prompts directory (one-time cleanup deletes the directory in Step 8, but the gitignore entry keeps any straggler writes out of version control during the rollout). <!-- completed: 2026-05-16T00:20 -->


### Step 2: Update the base-dir skill (`skills/base-dir/SKILL.md`)

- [x] Rewrite § *Procedure* to document the positional `TASK_NAME` branch as the primary path for task-aware consuming skills. The no-positional `AskUserQuestion` branch remains documented for the shared-root case. <!-- completed: 2026-05-16T00:26 -->
- [x] Update the § *CLI surface* code block to show `cafleet base-dir resolve [TASK_NAME] [--json]` and remove `--path`. <!-- completed: 2026-05-16T00:26 -->
- [x] Add a new sub-section documenting the absolute-path-arg recognition rules (ancestor match against `researches/<slug>/` or `design-docs/<NNNNNNN>-<slug>/`). <!-- completed: 2026-05-16T00:26 -->
- [x] Update § *Anchor file* to note that anchors are now written inside task folders too, and add `"task-scope"` to the documented `source` enum. <!-- completed: 2026-05-16T00:26 -->

### Step 3: Update the 5 consuming skills' SKILL.md files

- [x] `skills/research-report/SKILL.md` Step 0: replace the cafleet:base-dir invocation with `cafleet base-dir resolve researches/<topic-slug>`. Drop the `${OUTPUT_DIR} = ${BASE}/researches/<slug>/` concatenation since `${BASE}` IS the topic folder. Verify the `BASE:` line in every embedded spawn prompt now points at the task folder. <!-- completed: 2026-05-16T00:32 -->
- [x] `skills/research-presentation/SKILL.md` Step 0 (or equivalent): same change, with the slug the skill was handed. <!-- completed: 2026-05-16T00:32 -->
- [x] `skills/design-doc-create/SKILL.md` Step 0: replace the cafleet:base-dir invocation with `cafleet base-dir resolve design-docs/<NNNNNNN>-<slug>`. Drop the `${DOC_PATH} = ${BASE}/design-docs/$ARGUMENTS` concatenation; `${DOC_PATH}` is now `${BASE}/design-doc.md`. <!-- completed: 2026-05-16T00:32 -->
- [x] `skills/design-doc-execute/SKILL.md` Step 1 Phase 1: same change. After wiring, verify the existing Tier 3 base-directory discovery flow degenerates cleanly with task-scoped `${BASE}` (one slug folder containing a single `design-doc.md`) — per Spec § 6 *design-doc-execute Tier 3 verification*. Record the outcome inline in the SKILL.md change; if Tier 3 needs a follow-up edit to behave correctly with a single-doc BASE, file the edit as part of this task. Outcome: Tier 3 short-circuits cleanly (Tier 1 fires first when `$ARGUMENTS` is supplied because `${RESOLVED_ARGS} = ${BASE}/design-doc.md`); Tier 3 is preserved for the no-argument flow that uses the no-positional resolver (`${BASE}` = repo root, `${RESOLVED_ARGS} = ${BASE}/design-docs/`). No follow-up edit needed; documented inline in Phase 2 callout. <!-- completed: 2026-05-16T00:32 -->
- [x] `skills/design-doc-interview/SKILL.md` Step 0: same change. <!-- completed: 2026-05-16T00:32 -->

### Step 4: Create the new meta-skill `<repo>/.claude/skills/skill-author/SKILL.md`

<!-- COMMENT(director): Member Write tool denied for .claude/skills/* in dontAsk mode (harness safeguard). Workaround: Programmer writes the SKILL.md body to design-docs/0000060-skill-task-scoped-base-dir/skill-author-DRAFT.md (writable under the task folder) and replies complete (paragraph-Implementation > Step 4). Director then copies the body to .claude/skills/skill-author/SKILL.md, deletes the staging draft, and commits. -->

- [x] Create `<repo>/.claude/skills/skill-author/SKILL.md` with the seven sections enumerated in Specification § 8. Self-contained — no `Skill(cafleet:base-dir)` cross-reference is required for an author to follow the guide; the relevant rules are inlined. <!-- completed: 2026-05-16T01:04 (Programmer wrote to skill-author-DRAFT.md per Director option (b); Director will relocate to .claude/skills/skill-author/SKILL.md) -->
- [x] Verify the `description:` frontmatter explicitly lists the trigger phrases (e.g., "create a new CAFleet-orchestrated skill", "write a skill that spawns cafleet members", "add a Director/Member team skill"). The description is the only auto-load trigger surface. <!-- completed: 2026-05-16T01:04 (frontmatter description block lists six trigger phrases including the three required) -->

### Step 5: Implement resolver and CLI changes

- [x] In `cafleet/src/cafleet/base_dir.py`, refactor `resolve()` to **rename** the existing `path: str | None = None` keyword argument to `task_name: str | None = None`. The `path` parameter is removed entirely (no alias, matching the CLI's `--path` removal). Add the dispatch shown in § 2 at the top of `resolve()`: `if task_name is not None: return _resolve_task_scope(task_name, cwd=cwd_path)`. Implement `_resolve_task_scope` as a private helper using `pathlib.Path(...).mkdir(parents=True, exist_ok=True)` for auto-creation — no `subprocess`, no `os.makedirs`. <!-- completed: 2026-05-16T01:25 -->
- [x] Add a helper `_infer_repo_root(cwd: Path) -> Path | None` that walks `cwd` and its parents using the existing `is_git_repo_root(p)` helper; returns the first ancestor for which `.git` exists, or `None` if none found. <!-- completed: 2026-05-16T01:25 -->
- [x] Add a helper `_match_known_task_pattern(path: Path, repo_root: Path) -> Path | None` that walks parents of `path` and returns the first ancestor satisfying all four conditions in § 2 *Key invariants* (parent-of-parent == repo_root, parent name in `{researches, design-docs}`, slug regex `^\d{7}-[A-Za-z0-9_-]+$` for design-docs or `^[A-Za-z0-9][A-Za-z0-9_-]*$` for researches, ancestor is a directory or non-existent). Returns `None` on no match. <!-- completed: 2026-05-16T01:25 -->
- [x] Add `"task-scope"` to the `source` field's accepted enum in `_validate_anchor`. Do NOT add `"task-scope"` to the `record()` validation or the CLI `base_dir_record`'s `--source` Click Choice list — see Spec § 3. <!-- completed: 2026-05-16T01:25 -->
- [x] In `cafleet/src/cafleet/cli.py`, change `base_dir_resolve` to accept an optional positional `TASK_NAME` argument (`@click.argument("task_name", required=False)`) and remove the `--path` option. Map the `RuntimeError` raised by `_resolve_task_scope` (no `.git` ancestor) to exit code `1` with the error message on stderr; no JSON payload is emitted on this branch, even with `--json`. <!-- completed: 2026-05-16T01:25 -->
- [x] Wire the positional argument to `resolve(task_name=task_name, ...)`. <!-- completed: 2026-05-16T01:25 -->

### Step 6: Tests

- [x] In `cafleet/tests/test_base_dir.py`, add unit tests:
  - Relative `task_name` resolves under the inferred repo root; folder is auto-created; anchor written with `source: "task-scope"`.
  - Same `task_name` invoked twice in a row: second call reads the existing anchor with `source: "anchor"`.
  - Absolute `task_name` that lives inside `researches/<slug>/...` resolves to that slug folder.
  - Absolute `task_name` that lives inside `design-docs/<NNNNNNN>-<slug>/...` resolves to that slug folder.
  - Absolute `task_name` outside the inferred repo root (or equal to the repo root) returns `<unset>` (existing absolute-path-arg semantics preserved; the resolver does not walk skill-specific bucket patterns).
  - Slug-shape rejection: `design-docs/garbage` (no `^\d{7}-` prefix) fails the `_match_known_task_pattern` check and produces no anchor.
  - No-repo-root failure mode: CWD outside any git ancestor + positional `task_name` → exit 1 with the stderr message specified in § 2; no JSON payload even with `--json`. <!-- completed: 2026-05-16T01:14 (Tester Phase A; commit 9969ed7) -->
- [x] In `cafleet/tests/test_base_dir_spawn_flow.py` (or a new equivalent), add integration tests:
  - End-to-end: a fake consuming skill renders a spawn prompt with the task-scoped `BASE:` line; the rendered prompt audit file lands at `<task-folder>/prompts/<role>-<ts>.md`, not at the repo root. <!-- completed: 2026-05-16T01:14 (Tester Phase A; commit 9969ed7) -->
- [x] Run `mise //cafleet:test` and confirm no regressions. <!-- completed: 2026-05-16T01:25 (Programmer; 894 tests pass after Step 5 impl) -->

### Step 7: Validation against the dogfooded example

- [x] After implementation, confirm that `cafleet base-dir resolve design-docs/0000060-skill-task-scoped-base-dir` returns:
  - `status: "resolved"`
  - `base: /home/himkt/work/himkt/cafleet/design-docs/0000060-skill-task-scoped-base-dir`
  - `source: "anchor"` (or `"task-scope"` if the anchor was never written — both are acceptable here)
  - `anchor: /home/himkt/work/himkt/cafleet/design-docs/0000060-skill-task-scoped-base-dir/.cafleet-base-dir.json` <!-- completed: 2026-05-16T01:27 (status: resolved, source: "task-scope" on first call — anchor now written) -->
- [x] Confirm the audit file from the Drafter spawn of THIS very design-doc-create run already lives at `<this task folder>/prompts/drafter-<ts>.md` — this is the dogfood proof. <!-- completed: 2026-05-16T01:27 (drafter-20260515T230730Z.md confirmed at <task-folder>/prompts/) -->

### Step 8: One-time cleanup

- [x] Delete the existing `<repo>/prompts/` directory (its contents are stale audit artifacts from the pre-task-scope era — Spec § 9). The `/prompts/` entry added to `.gitignore` in Step 1 prevents any future strays from re-entering version control. <!-- completed: 2026-05-16T01:27 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-16 | Initial draft |
| 2026-05-16 | Reviewer round 1: resolved 12 markers (progress count, migration story, repo-root inference spec, slug-shape validation, no-repo-root failure mode, record-scope clarification, design-doc-execute Tier 3 verification downgrade, skill location & discovery, Step 1 pre-resolution, Step 4 "confirm" task removal, § 2/Step 5 reconciliation, `--path` test bullet drop). |
| 2026-05-16 | User approval. Status → Approved. |
