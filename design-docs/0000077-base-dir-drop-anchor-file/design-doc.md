# 0000077 — Drop the `.cafleet-base-dir.json` anchor file

**Status**: Complete
**Progress**: 18/18 tasks complete
**Last Updated**: 2026-06-07

## Overview

The `cafleet-base-dir` skill writes a `.cafleet-base-dir.json` "anchor" file to disk every time it resolves `${BASE}`, to cache the result across reloads. This document removes the anchor mechanism entirely, because BASE resolution is deterministic in both non-interactive branches (Step 0 → task folder, Step 1 → CWD), so the on-disk cache adds no value. Predecessor 0000074 removed the `cafleet base-dir` CLI and folded resolution into this skill while keeping the anchor; this change drops the anchor too.

## Success Criteria

- [x] `skills/cafleet-base-dir/SKILL.md` contains zero mentions of `.cafleet-base-dir.json`, the anchor, the `version: 1` validation, the `source` field, `source: anchor` reporting, `resolved_at`, the "Anchor file" schema section, or the "Gitignore handling" section.
- [x] The skill's four "deterministic guarantees" are reduced to two: guarantee 1 (repo-root inference) and guarantee 2 (traversal-escape + repo-root-degenerate rejection) survive, folded inline into Step 0's numbered steps; guarantees 3 and 4 are gone; no standalone "Deterministic guarantees" preamble remains.
- [x] Step 0, Step 1, and Step 2 perform no anchor read or write; Step 2 (`AskUserQuestion`) is retained but no longer persists the chosen answer.
- [x] The `<unset>` sentinel contract and the no-bypass write protocol survive (trimmed, semantics intact); every consuming skill and `skills/cafleet/reference/director.md` remain accurate against them.
- [x] `.gitignore` no longer contains the `.cafleet-base-dir.json` entry or its comment; the `/prompts/` and per-role audit entries stay.
- [x] The five blast-radius docs/skills are trimmed of anchor wording: `member-lifecycle.md`, `cafleet-research-report`, `cafleet-design-doc-execute`, `cafleet-research-presentation`, `.claude/skills/skill-author`.
- [x] No Python source change: `cafleet/src/cafleet/cli.py` is untouched (its only base-dir reference, the `--prompt-file` error string at `cli.py:694`, names the *skill*, not the anchor, and stays).
- [x] A repo-wide search for `.cafleet-base-dir.json` and for "anchor" in the base-dir-file sense returns zero hits outside `design-docs/` and git history (legitimate non-base-dir "anchor" uses are enumerated in the Sweep note's do-not-touch list).
- [x] `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:test` pass (sanity gate — no code change is expected).

---

## Background

The `cafleet-base-dir` skill is the single authoritative resolver for every CAFleet scratch / audit / figure / spawn-prompt path. It resolves `${BASE}` through three branches:

| Branch | Trigger | Outcome |
|---|---|---|
| **Step 0** (task-scope) | Consumer operates on a per-task folder | `${BASE}` = the task folder (e.g. `design-docs/<slug>`) |
| **Step 1** (shared-root) | No per-task folder convention | `${BASE}` = the CWD, unless CWD is `$HOME` / `~/.claude` |
| **Step 2** (`AskUserQuestion`) | Step 1 lands in `$HOME` or `~/.claude` | User picks `/tmp/claude-code`, the CWD, or a custom path |

Today, every branch writes a `.cafleet-base-dir.json` anchor recording `{version, base, source, resolved_at}` so that re-loading the skill (after auto-compression, a fresh Director, or the next session) returns the same answer without re-running the procedure. Reading an anchor enforced two of the four "deterministic guarantees" (guarantee 3: `version: 1` validation; guarantee 4: idempotent record-on-mismatch).

**Why the anchor is unnecessary.** Resolution is deterministic in both non-interactive branches: Step 0 maps a task-folder relpath to the same absolute folder every time, and Step 1 maps a working directory to itself. Re-running them always yields the same answer, so caching it on disk buys nothing. The anchor only ever mattered to remember the **interactive** `AskUserQuestion` choice (Step 2) — which fires solely when CWD is `$HOME` or `~/.claude`. Within a session that distinction is moot anyway: the Director resolves `${BASE}` once and bakes it into spawn prompts as a literal absolute path, and members never re-resolve (the no-bypass write protocol already forbids member re-resolution). Dropping the anchor means the rare Step 2 branch re-prompts on reload instead of reading a cached answer — an acceptable cost (see Risks).

---

## Specification

### Removed

- The anchor **write** (Step 0 step 5, Step 1 branch 2, and the Step 2 persistence tail; Step 1 branch 3 performs no write — its candidate-anchor read is covered by the read-back bullet below).
- The anchor **read-back** and its `version: 1` validation (guarantee 3).
- The idempotent **record-on-mismatch** logic (guarantee 4).
- The standalone **"Deterministic guarantees"** preamble (the four-item list) — guarantees 1 & 2 move inline into Step 0; 3 & 4 are gone.
- The entire **"Anchor file"** section (path, schema, `version`, `base`, `source`, `resolved_at`).
- The entire **"Gitignore handling"** section.
- The **`source`** field concept and all `source: anchor` / `source: task-scope` / `source: cwd-inference` / `source: askuserquestion` reporting (the field lived only in the anchor).
- The `.cafleet-base-dir.json` entry (and its comment) in `.gitignore`.

### Kept (trimmed)

- The skill's framing as the single authoritative resolver; `${BASE}` is the only legitimate write root; consumers MUST NOT compute `${BASE}` independently nor fall back to `/tmp` on `<unset>`. `/tmp/claude-code` stays a valid resolved `${BASE}` when Step 2 selects it.
- Step 0 / Step 1 / Step 2 (rewritten without anchor handling).
- The consumer-contract canonicalization table (`design-docs/<slug>`, `researches/<topic-slug>`).
- The no-bypass write protocol (4 items) — prose trimmed, semantics intact.
- The `<unset>` sentinel section (3 items) — prose trimmed, semantics intact.

### New resolution procedure

The skill resolves `${BASE}` using only `git rev-parse --show-toplevel` (Bash) and `AskUserQuestion`. It writes nothing at resolution time — the task folder is created lazily by the first consumer write (the `Write` tool auto-creates parent directories), so no `Read`/`Write` of an anchor is involved.

**Step 0 — Task-scope resolution** (guarantees 1 & 2 folded in):

1. **Infer the repo root** (guarantee 1): run `git rev-parse --show-toplevel`. Empty / non-zero exit → STOP and tell the user to `cd` to the repo root and retry.
2. **Canonicalize** `$ARGUMENTS` to the task-folder relpath per the consumer's convention (table below).
3. **Guard the path** (guarantee 2):
   - **Relative `task_name`**: join under the repo root and resolve. Equals the repo root → STOP ("the repo root is not a task folder"). Not under the repo root (a `..` escape) → STOP ("refusing to create a task folder outside the repo").
   - **Absolute `task_name`**: resolve it. Equals the repo root OR not strictly under it → `${BASE} = <unset>`; create nothing.
4. `${BASE}` = the absolute task folder. The folder is created lazily on the first consumer write; there is no resolution-time write.

**Step 1 — Shared-root resolution** (no task name):

1. Determine the CWD and `$HOME`; let `claude_subdir = $HOME/.claude`.
2. If the CWD is NOT `$HOME` and NOT `claude_subdir` (nor under it) → `${BASE}` = the CWD. Done.
3. Otherwise (CWD is `$HOME` or under `~/.claude`) → go to Step 2 with candidates `[/tmp/claude-code, <CWD>]`.

**Step 2 — `AskUserQuestion`** (only when Step 1 reaches branch 3):

Present the candidates ("Select the base directory for output files:"):
- `/tmp/claude-code (recommended)` → `${BASE} = /tmp/claude-code`
- `${CWD}` → `${BASE} = ${CWD}`
- `Other` (free text) → `${BASE} = user input` (resolve against `${CWD}` if relative)

The chosen value is `${BASE}`. Nothing is persisted; if the skill reloads in the same `$HOME` / `~/.claude` CWD, this branch re-prompts (accepted — see Risks).

### Documentation surface / blast radius

Per the project documentation-first rule, the skill and docs land before any other change. (There is no code change here; the "Verify" step only confirms that.)

**Trim (anchor wording verified present):**

| File | Change |
|---|---|
| `skills/cafleet-base-dir/SKILL.md` | Full rewrite per *New resolution procedure*: remove the anchor mechanism, fold guarantees 1 & 2 into Step 0, delete the "Anchor file" + "Gitignore handling" sections, dedupe the surviving prose, update the tool list to `git rev-parse` (Bash) + `AskUserQuestion`. |
| `.gitignore` | Remove the `.cafleet-base-dir.json` entry and its 3-line comment (currently lines 35–38). Keep the `# ==== CAFleet runtime artifacts at the repo root ====` section header (line 34) and the `/prompts/` + per-role audit entries (audit writes still happen). |
| `docs/concepts/member-lifecycle.md` | "Base-dir resolution" section: drop the anchor read/write description, the idempotent-anchor sentence, and the entire `version`-lock / `resolved_at` paragraph. Keep CWD-inference, the task-scope flow (reworded: resolves to the abs folder, created lazily), and the `<unset>` paragraph. |
| `skills/cafleet-research-report/SKILL.md` | Step 0 (≈ line 62): drop the "writes `<task-folder>/.cafleet-base-dir.json` with `source: "task-scope"` (or reads an existing anchor)" clause and the "auto-creates the folder" claim; reword to "joins the relpath under the repo root and resolves `${BASE}` to that absolute folder". |
| `skills/cafleet-design-doc-execute/SKILL.md` | Step 1 Phase 1 (≈ line 230): drop "persist the answer by writing the anchor" — Step 2 simply yields `${BASE}` and the Director re-resolves on reload. |
| `skills/cafleet-research-presentation/SKILL.md` | Step 0 (≈ lines 74/78): reword the "create and anchor a `report.md` directory" / "anchor" verb so it no longer implies an anchor file (e.g. "create a `report.md` directory"). |
| `.claude/skills/skill-author/SKILL.md` | The reproduced procedure (≈ lines 70–71): drop the per-task anchor write/read description and the `source: "task-scope" \| "anchor"` outcome; reword line ≈ 75 "(or equals the repo root, which would clobber the shared-root anchor)" → "(or equals the repo root)"; reword line ≈ 184 "drift from the Director's anchor" → "drift from the Director's resolved BASE"; reword line ≈ 343 "(auto-created + anchored, source: task-scope)" → "(auto-created)". |

**Sweep (catch residuals):** grep every consuming skill for residual anchor-file wording — `.cafleet-base-dir.json`, "writing the anchor" / "write the anchor", `source: anchor`, `source: "task-scope"`, `resolved_at`, and "anchor" used in the base-dir-file sense — and trim. **Do not touch** these legitimate, non-base-dir "anchor" uses (this is the allowed-"anchor" set the Verify grep relies on): the coordination-protocol terms "Anchorless status" / "source-anchored" / "design-doc-anchored" / "line-anchored" (marker-pointer concepts, not the anchor file); matplotlib's `bbox_to_anchor`; "date-anchored" search patterns (`skills/cafleet-research-report/SKILL.md:390`, `roles/scout.md`, `roles/researcher.md`); "manifests anchor at the same `skills/` directory" (`docs/concepts/overview.md:166`); "role-specific anchor" (`skills/cafleet/roles/member.md:5`, `roles/director.md:5`); and the `audit-disabled` anchorless status (`skills/cafleet/reference/director.md:68`).


**No change (verified accurate post-removal):**

| File | Why it stays |
|---|---|
| `skills/cafleet/reference/director.md` | References BASE resolution generically + the `<unset>` sentinel + the `audit-disabled` anchorless status (a coordination concept). All accurate once the anchor is gone. |
| `skills/cafleet-design-doc/guidelines.md` | "The `design-docs/` prefix is auto-prepended via the `cafleet-base-dir` skill integration" — the consumer-side canonicalization survives. Accurate. |
| `skills/cafleet-design-doc-create/SKILL.md`, `skills/cafleet-design-doc-interview/SKILL.md` | Their Step 0 says "run the skill's Step 0 (task-scope resolution)" — Step 0 still exists. Their "Anchorless Status" sections are the coordination concept, not the anchor file. |
| `skills/cafleet-create-figure/SKILL.md` | `bbox_to_anchor` is matplotlib; "Load the `cafleet-base-dir` skill" stays. |
| `README.md`, `docs/get-started/configure.md`, `docs/spec/cli-options.md` | Name the skill only. |
| `cafleet/src/cafleet/cli.py` | `cli.py:694` names the skill in the `--prompt-file` error string; no anchor I/O exists in source. No Python change. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Documentation-first: the skill + docs (Steps 1–3) land before the verify pass (Step 4).

### Step 1: Rewrite the `cafleet-base-dir` skill

- [x] Remove the standalone "Deterministic guarantees" preamble; fold guarantee 1 (repo-root inference) and guarantee 2 (traversal-escape + repo-root-degenerate rejection) into Step 0's numbered steps <!-- completed: 2026-06-07T08:39 -->
- [x] Rewrite Step 0 (task-scope): infer repo root, canonicalize, guard the path, set `${BASE}` = abs task folder created lazily on first write; remove the anchor resolve/write step <!-- completed: 2026-06-07T08:39 -->
- [x] Rewrite Step 1 (shared-root): branch 2 returns the CWD directly; branch 3 goes straight to Step 2 — no anchor read/write <!-- completed: 2026-06-07T08:39 -->
- [x] Rewrite Step 2 (`AskUserQuestion`): yield `${BASE}` from the choice; remove the "persist the answer by writing the anchor" tail <!-- completed: 2026-06-07T08:39 -->
- [x] Delete the "Anchor file" and "Gitignore handling" sections; remove every `source` / `source: anchor` / `version` / `resolved_at` mention; rewrite the Procedure intro — drop the `Read, Write` tools (→ `git rev-parse` (Bash) + `AskUserQuestion`), drop clause "(c) persist the anchor with the Write tool", and drop the historical "the deterministic guarantees the resolver once enforced in compiled Python are preserved below" framing <!-- completed: 2026-06-07T08:39 -->
- [x] Trim the no-bypass write protocol and the `<unset>` sentinel section (keep semantics; dedupe overlapping admonitions); within this, reword no-bypass item 3 (SKILL.md:110) "drift if the Director's anchor changed mid-session" → "drift if the Director's resolved BASE changed mid-session" <!-- completed: 2026-06-07T08:39 -->
- [x] Reword the surviving base-dir-sense "anchor" mention in the KEPT consumer-contract note (SKILL.md:49) "...and anchors the wrong BASE" → "...and resolves the wrong BASE" <!-- completed: 2026-06-07T08:39 -->


### Step 2: Trim the blast-radius docs/skills

- [x] `docs/concepts/member-lifecycle.md`: trim the "Base-dir resolution" section (drop anchor read/write + idempotency + the `version`-lock / `resolved_at` paragraph; keep CWD-inference, task-scope reworded, and `<unset>`) <!-- completed: 2026-06-07T08:48 -->
- [x] `skills/cafleet-research-report/SKILL.md`: trim Step 0 anchor wording (drop the `.cafleet-base-dir.json` write + "auto-creates the folder" claim) <!-- completed: 2026-06-07T08:48 -->
- [x] `skills/cafleet-design-doc-execute/SKILL.md`: drop "persist the answer by writing the anchor" in Step 1 Phase 1 <!-- completed: 2026-06-07T08:48 -->
- [x] `skills/cafleet-research-presentation/SKILL.md`: reword the "anchor" verb in Step 0 <!-- completed: 2026-06-07T08:48 -->
- [x] `.claude/skills/skill-author/SKILL.md`: drop the anchor write/read from the reproduced procedure; reword "drift from the Director's anchor" and "(auto-created + anchored, source: task-scope)" <!-- completed: 2026-06-07T08:51 -->
- [x] Sweep every consuming skill for residual anchor-file wording and trim; leave coordination-protocol "Anchorless"/"source-anchored" terms and `bbox_to_anchor` untouched <!-- completed: 2026-06-07T08:51 -->

### Step 3: Remove the `.gitignore` anchor entry

- [x] Delete the `.cafleet-base-dir.json` entry + its 3-line comment; keep the section header and the `/prompts/` + per-role audit entries <!-- completed: 2026-06-07T09:00 -->
- [x] Sweep the working tree for any lingering `.cafleet-base-dir.json` files (repo root, `researches/<slug>/`, `design-docs/<NNNNNNN>-<slug>/`) and delete them, or confirm none exist — so dropping the ignore pattern does not surface stale anchors as untracked (per `rules/removal.md`) <!-- completed: 2026-06-07T09:04 -->


### Step 4: Verify

- [x] Confirm no Python change: `cafleet/src/cafleet/cli.py` is untouched (the `cli.py:694` error string still names the skill) <!-- completed: 2026-06-07T09:08 -->
- [x] Repo-wide search for `.cafleet-base-dir.json` and base-dir-sense "anchor" returns zero hits outside `design-docs/` and git history (treat the Sweep note's do-not-touch list as the allowed-"anchor" set) <!-- completed: 2026-06-07T09:08 -->
- [x] `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test` pass (sanity gate — no code change expected) <!-- completed: 2026-06-07T09:08 -->

---

## Risks / Trade-offs

1. **`AskUserQuestion` re-prompts on reload (accepted).** Without the anchor, the Step 2 branch can no longer cache its answer, so it re-prompts whenever the skill reloads while the CWD is `$HOME` or under `~/.claude`. This branch is already rare (it never fires from a normal project CWD), and within a session the Director resolves `${BASE}` once and bakes it into spawn prompts as a literal — members never re-resolve. The re-prompt is therefore bounded to a Director re-running the skill from a home-dir CWD across reloads. The user explicitly chose to keep the branch and accept this.
2. **Loss of cross-session per-task anchor persistence (accepted).** No consumer genuinely relies on it: task-scope re-resolution is deterministic (the same relpath maps to the same folder), and shared-root re-resolution returns the CWD deterministically. The only readers of an existing anchor (`cafleet-research-report`, the `skill-author` description) treated it as a pure optimization, not a correctness dependency. Net impact: none.
3. **`version`-lock concern dissolves.** Guarantee 3 existed so two installations at different cafleet versions could not disagree about a cached BASE. With nothing cached, each resolution is fresh, so there is no stored value to disagree about — removing the guard removes the problem it guarded against rather than weakening anything.

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-07 | Initial draft |
| 2026-06-07 | Addressed Reviewer markers: corrected the WRITE-bullet branch list (Step 1 branch 2, not 2 & 3); added skill-author:75 + the literal `.gitignore` header; enumerated the legitimate non-base-dir "anchor" do-not-touch set; added resolver-skill residual rewords (SKILL.md:16/49/110) and the on-disk anchor-artifact cleanup task |
| 2026-06-07 | User approved; Status → Approved. Spec frozen; ready for implementation (0/18 tasks) |
| 2026-06-07 | Implementation complete (18/18 tasks); all success criteria verified; lint/typecheck/662 tests green |
| 2026-06-07 | Status → Complete (PR #99 review addressed) |
