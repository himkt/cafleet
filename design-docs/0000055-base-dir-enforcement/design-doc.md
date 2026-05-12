# Base-dir Authoritative Resolution

**Status**: Approved
**Progress**: 21/23 tasks complete
**Last Updated**: 2026-05-12

## Overview

Make `Skill(cafleet:base-dir)` the single authoritative resolver for every CAFleet scratch / audit / figure path. Today five bypass paths let consumer skills write to `/tmp` without base-dir's consent; this design eliminates the bypasses, persists the resolved BASE in an on-disk anchor so reloads are idempotent across context compaction, and ships a small Python+CLI helper so the contract is unit-testable. `/tmp/claude-code` remains a valid resolved BASE when base-dir explicitly selects it — only **bypassing** base-dir is forbidden.

## Success Criteria

- [ ] `cafleet base-dir resolve` / `cafleet base-dir record` CLI subcommands exist, are unit-tested, and round-trip the anchor file cleanly.
- [ ] `Skill(cafleet:base-dir)` resolution is deterministic + idempotent: a second `Skill(cafleet:base-dir)` call (same CWD, same anchor) returns the same BASE without re-prompting via `AskUserQuestion`.
- [ ] Absolute-path callers receive `BASE=<unset>` (explicit sentinel); any code that tries to derive a path from `<unset>` errors loudly rather than silently no-op or falls back to `/tmp`.
- [ ] All 5 consuming skills (research-report, research-presentation, design-doc-create, design-doc-execute, design-doc-interview) substitute the resolved BASE into every spawn-prompt template they emit. Members never have to re-resolve BASE on their own (compaction-safe).
- [ ] All 12 *member* role files under `skills/*/roles/` (Drafter, Reviewer, Manager, Scout, Researcher, Presentation, Transcript, Visual Reviewer, Programmer, Tester, Verifier, Analyzer) load `Skill(cafleet:base-dir)` on startup, picking up the shared no-bypass protocol from a single source of truth. The 4 Director role files (`research-report/roles/director.md`, `research-presentation/roles/director.md`, `design-doc-create/roles/director.md`, `design-doc-execute/roles/director.md`) are excluded — a Director IS main-Claude and the consumer SKILL.md already loads `Skill(cafleet:base-dir)` in its Step 0 flow before any spawn, so loading it a second time inside the role file is redundant.
- [ ] `create-figure` no longer overrides BASE to `/tmp/claude-code` for git-repo-root CWDs; figures land under `BASE/figures/{src,output,data}` whatever BASE is (including `/tmp/claude-code` when base-dir picks it).
- [ ] Integration test: with a non-`/tmp` BASE anchor in place, drive the Director-side audit-file write helper for at least one role; verify the audit lands under `${BASE}/<role>.md` and zero files are written under any `/tmp/` subtree. Real-member end-to-end coverage is manual-smoke only (§Out of Scope).
- [ ] `ARCHITECTURE.md`, `README.md`, every affected `SKILL.md`, and every affected role file reflect the new contract in the same change. No "deprecated" notices remain (per `~/.claude/rules/removal.md`).

---

## Background

Six failure modes let `/tmp` writes happen without (or against) base-dir's consent. Reframed under user guidance: only Mode A is an authorized `/tmp` choice; modes B–F are bypasses to eliminate.

| Mode | Location | Verdict | Fix lives in |
|:--|:--|:--|:--|
| A — `/tmp/claude-code (recommended)` is first AskUserQuestion option | `skills/base-dir/SKILL.md` L15–18 | **STAYS** (user authorized) | n/a |
| B — `create-figure` unconditionally overrides BASE to `/tmp/claude-code` for git-repo-root CWDs | `skills/create-figure/SKILL.md` L24–27 | BYPASS — fix | §Specification 1 |
| C — 5 consumer skills never substitute BASE into spawned-member prompts | `skills/{research-report,research-presentation,design-doc-create,design-doc-execute,design-doc-interview}/SKILL.md` | BYPASS — fix | §Specification 2 |
| D — 12 role files are silent on the no-bypass rule | `skills/*/roles/*.md` | BYPASS — fix | §Specification 3 |
| E — BASE lives only in main-Claude context, lost on auto-compression | architectural | BYPASS — fix | §Specification 4 |
| F — Absolute-path argument leaves BASE undefined; downstream gating silently no-ops | `skills/base-dir/SKILL.md` L12 | BYPASS — fix | §Specification 5 |

Two surrounding facts that informed the design:

- The canonical no-`/tmp` rule is stated identically in 5 consumer `SKILL.md` files (research-report L115, research-presentation L108, design-doc-create L274, design-doc-execute L375, design-doc-interview L148) but is NOT mirrored in any of the 12 member role files. The role files are unreachable from main-Claude's context once spawned — so the only way to teach them is to centralize the rule in `skills/base-dir/SKILL.md` and have every role file load it.
- There is no existing `base_dir` Python code in the repo; resolution is purely SKILL.md instructions to a Claude agent. That is why none of the existing failure modes have unit-test coverage. Introducing a thin Python helper unlocks the test surface required by Q9.

---

## Specification

### 1. `create-figure` git-repo-root override removed (Mode B)

Replace `skills/create-figure/SKILL.md` §"Resolve `${BASE}` in this order" step 2:

| Before | After |
|:--|:--|
| If the resolved `${BASE}` is a git repository root, override to `${BASE} = /tmp/claude-code` so generated figures do not pollute the repo tree. | Use the resolved BASE verbatim. Figures, scripts, and data land under `${BASE}/figures/{src,output,data}` regardless of whether BASE is a git-repo root, `/tmp/claude-code`, or any other path. If the user wants figures kept out of a repo tree, they choose `/tmp/claude-code` (or any non-repo path) at base-dir resolution time — `create-figure` no longer second-guesses. |

The example resolution paragraph (L35) is updated to match. The "Never create scripts or outputs directly in a project repo root" sentence (L39) is replaced with: "Figure artifacts always live under `${BASE}/figures/`; never directly at `${BASE}` and never at `/tmp` unless `${BASE}` itself is `/tmp/claude-code`."

### 2. Spawn prompts substitute the resolved BASE (Mode C)

The five consumer skills construct spawn prompts by reading a SKILL.md-embedded template and substituting placeholders. CAFleet uses two distinct substitution mechanisms: curly-brace tokens (`{session_id}` / `{agent_id}` / `{director_agent_id}`) are substituted by `cafleet member create` via `str.format()` at member-spawn time, and `[INSERT …]` markers are substituted Director-side in shell BEFORE the `cafleet member create` call. This design uses the second mechanism: each affected spawn template gains a new `[INSERT BASE]` marker (formally `[INSERT abs BASE path]`) that the Director replaces with the literal absolute BASE path before invoking `cafleet member create`. Using the `[INSERT …]` convention sidesteps the Template-safety rule (`design-doc-create/SKILL.md` L280: "any literal `{` / `}` you embed in a custom prompt must be doubled") and avoids collision with `coding_agent.py`'s `str.format()` substitution pass.

Spawn templates live in the 5 consumer SKILL.md files (not in `roles/*.md` files). Each consumer SKILL.md hosts one or more spawn templates per role-type — the Drafter alone has two (normal-mode template at `design-doc-create/SKILL.md` L282–306 and resume-mode template at L312–334), so the strict template count is greater than 12. The total surface, covering 12 role spawns across both normal and resume modes where applicable:

| Consumer skill | Roles with spawn templates that must accept `[INSERT BASE]` |
|:--|:--|
| `research-report` | Manager, Scout, Researcher |
| `research-presentation` | Presentation, Transcript, Visual Reviewer |
| `design-doc-create` | Drafter (normal + resume modes), Reviewer |
| `design-doc-execute` | Programmer, Tester, Verifier |
| `design-doc-interview` | Analyzer |

**Insertion position.** The `BASE: …` line is inserted immediately after the `YOUR AGENT ID: {agent_id}` line in every affected spawn template. This keeps the line at a stable position near the top of the prompt, well clear of the tmux `command too long` budget (~2 KB, see `design-doc-create/SKILL.md` L272), and groups it with the other identity / context anchors the member relies on for its first turn.

Each consumer's SKILL.md spawn-prompt section gains one line in that fixed position:

```
BASE: [INSERT abs BASE path the Director resolved via Skill(cafleet:base-dir)]
```

When the Director's own BASE is `<unset>` (absolute-path argument branch), spawned members still operate normally because their actual work target is already an absolute path baked into the spawn prompt — e.g., the Drafter receives `[INSERT abs design-doc directory]` (the resolved `design-docs/0000NNN-foo/` path) and the Researcher receives `[INSERT abs research folder]` (the resolved `researches/topic-slug/` path). Audit-file writes (a Director-side operation per `design-doc-create/SKILL.md:350`, NOT a member-side one) are guarded-skipped on the Director side per §Specification 5 item 1. The spawn prompt MUST omit the `BASE:` line entirely whenever the Director's BASE is `<unset>` — the literal string `BASE: <unset>` is never written into any spawn prompt.

### 3. Role files load `Skill(cafleet:base-dir)` (Mode D)

Each of the 12 role files gains, in its "Load at startup" instruction block, a new line:

```
Load these skills at startup:
- Skill(cafleet:base-dir) — for the no-bypass write protocol and BASE-derived path conventions
- Skill(cafleet) — for communication with the Director
- Skill(design-doc) — (where applicable)
```

The canonical no-bypass write protocol is moved into `skills/base-dir/SKILL.md` (§ *No-bypass write protocol*, new section). The five existing consumer-`SKILL.md` "Scratch and audit files — never `/tmp`" blurbs (research-report, research-presentation, design-doc-create, design-doc-execute, design-doc-interview) are replaced with a single one-line pointer: "See `Skill(cafleet:base-dir)` § *No-bypass write protocol*." Per `~/.claude/rules/removal.md`, the duplicated text is removed in the same change — no deprecation pointers.

The new `skills/base-dir/SKILL.md` § *No-bypass write protocol* states:

1. Every scratch / audit / figure / spawn-prompt-render write MUST land under `${BASE}` or under an explicit consumer-supplied absolute target — e.g., the design-doc directory delivered to spawned members via `[INSERT abs design-doc directory]`, or the research folder delivered via `[INSERT abs research folder]`. Never `/tmp` unless `${BASE}` itself is `/tmp/claude-code`.
2. If `${BASE}` is the sentinel `<unset>` (absolute-path argument branch), any code that tries to compute a path from BASE MUST abort with `Error: BASE is <unset>; refusing to fall back to /tmp` (loud failure).
3. The resolved BASE is delivered to spawned members via the Director's spawn-prompt substitution. Members MUST NOT re-resolve BASE on their own; they MUST use the literal path their spawn prompt baked in.
4. If a member's BASE line is missing from its spawn prompt, the member treats the audit-file feature as disabled and emits a single CAFleet message back to the Director as a parens-free anchorless status (per `skills/design-doc/coordination.md` § *Anchorless Status*): `audit-disabled no BASE in spawn prompt`. The phrasing deliberately omits parentheses so a Director reading the broker log does not misinterpret it as a malformed `<verb> (<pointer>)` hop. The member MUST NOT fall back to `/tmp`.

### 4. Anchor file for idempotent resolution (Mode E)

A new on-disk anchor file persists the resolved BASE so that re-loading `Skill(cafleet:base-dir)` returns the same answer without re-prompting via `AskUserQuestion`. This covers the auto-compression case and the "main-Claude resumes after restart" case.

**File location**: `${BASE}/.cafleet-base-dir.json`

**Gitignore handling.** When `${BASE}` is a git-repo CWD (the normal case in this project), the anchor file will surface in `git status`. Decision:

1. The cafleet repo's own `.gitignore` adds `/.cafleet-base-dir.json` at repo root, so the file never appears as untracked here. (Implementation Step 1 task.)
2. `cafleet base-dir record` does NOT mutate any `.gitignore` — modifying a user repo's gitignore as a side-effect would be a surprising write.
3. `skills/base-dir/SKILL.md` and `ARCHITECTURE.md` document the recommendation: users who do not want anchor noise in arbitrary repos should add `.cafleet-base-dir.json` to their global excludes (`~/.config/git/ignore` or `core.excludesFile`). The first run of `cafleet base-dir record` prints a one-line stderr tip pointing at this recommendation when it has just written a new anchor under a git-repo BASE.

**Format**:

```json
{
  "version": 1,
  "base": "/tmp/claude-code",
  "source": "askuserquestion",
  "resolved_at": "2026-05-12T13:00:00+00:00"
}
```

| Field | Type | Notes |
|:--|:--|:--|
| `version` | int | Schema version, starts at 1. Behavior on mismatch is fail-loud (see *Version handling* below). |
| `base` | string | Absolute path; MUST equal the directory the anchor lives in. Mismatch is a fatal error (see Resolution Procedure). |
| `source` | enum | One of `"cwd-inference"`, `"askuserquestion"`. Records which branch produced this anchor. |
| `resolved_at` | ISO 8601 | UTC, microsecond precision, `+00:00` suffix (same convention as cafleet `status_timestamp`). |

**Version handling.** `cafleet base-dir resolve` rejects any anchor whose `version` field is missing, not a positive integer, or not equal to 1: it exits with `Error: anchor at <path> has version=<N>; this cafleet supports version 1. Delete the anchor and re-resolve.` This is uniform across `version == 0`, absent, and `version > 1` (forward-incompatible). The policy is intentional — silent tolerance of unknown future versions would let two installations on the same machine at different cafleet versions silently disagree about BASE. When this design ships, every existing anchor (there are none in the wild yet, but a clean break is asserted by user Q10) is by definition version 1. A future schema bump will land alongside an explicit upgrade path; this design does NOT implement one.

**Resolution Procedure** (replacing `skills/base-dir/SKILL.md` § Procedure):

1. **Absolute-path argument** → return sentinel `BASE=<unset>`. No anchor read, no anchor write. Done.
2. **CWD-deterministic branch**: if `${CWD} != $HOME` and `${CWD}` is not under `$HOME/.claude`:
   - Candidate `BASE = ${CWD}`.
   - If `${CWD}/.cafleet-base-dir.json` exists and its `base` field equals `${CWD}` → use it. Done.
   - If `${CWD}/.cafleet-base-dir.json` exists and its `base` field mismatches → fatal error: `Error: anchor file <path> records base=<X> but lives at <Y>; refusing to use.`
   - Otherwise → write the anchor with `source="cwd-inference"`. Use it. Done.
3. **AskUserQuestion branch** (`${CWD} == $HOME` or under `$HOME/.claude`):
   - Probe known candidate anchors in order: `/tmp/claude-code/.cafleet-base-dir.json`, `${CWD}/.cafleet-base-dir.json`. The first that exists AND is internally consistent (its `base` field equals its parent directory) wins.
   - If no anchor is found → run `AskUserQuestion` with the existing three options (`/tmp/claude-code (recommended)` / `${CWD}` / `Other`), resolve `${BASE}`, then write the anchor with `source="askuserquestion"`.
   - Note: a `/tmp/claude-code/.cafleet-base-dir.json` anchor does not survive reboot — acceptable. After reboot, the AskUserQuestion fires once and re-creates the anchor.

The Director's substitution into spawn prompts (§Specification 2) does the cross-pane delivery; the anchor file does the cross-compaction recovery for the Director itself. A member that hits compaction mid-task simply re-reads its spawn prompt notes (BASE is already baked in there) — it never reads the anchor.

### 5. Sentinel for absolute-path branch (Mode F)

`Skill(cafleet:base-dir)` is the only place that may return `BASE=<unset>`. The sentinel is a literal string `"<unset>"` (case-sensitive). Any consumer skill that has BASE set to `<unset>` MUST:

1. Audit-file writes that derive their path from BASE are guarded by an explicit `BASE != <unset>` check at the call site. The guard is mandatory; reaching `Path(BASE) / …` without the guard is the failure mode item 3 catches. A guarded skip is the intended path when BASE is `<unset>` — no silent no-op, no fallback to `/tmp`, just an explicit skip whose condition is visible in the source.
2. NOT spawn members with `BASE: <unset>` in the spawn prompt — the line is omitted entirely.
3. If a code path under `BASE=<unset>` reaches an unguarded `Path(BASE) / …` computation, abort with the standardized error: `Error: BASE is <unset>; refusing to fall back to /tmp` (see §Specification 3 protocol item 2).

Skip (item 1) and loud-error (item 3) are different stages of the same protocol, not alternative resolutions: every BASE-dependent write site MUST be guarded; the loud error is the safety net for sites that forgot to guard. The current silent-skip behavior masked the bypass bugs this design exists to fix.

### 6. Python helper + CLI surface (testability for Q9)

A new module `cafleet/src/cafleet/base_dir.py` and two new CLI subcommands implement the deterministic side of resolution (everything except `AskUserQuestion`, which can only be invoked from Claude's tool context).

```bash
# Probe-only resolution. Does NOT prompt. Reports what should happen.
cafleet base-dir resolve [--path <abs-or-rel-arg>] [--json]

# Possible outputs (JSON form):
# {"status": "resolved", "base": "/home/user/proj", "source": "cwd-inference", "anchor": "/home/user/proj/.cafleet-base-dir.json"}
# {"status": "resolved", "base": "/tmp/claude-code", "source": "anchor", "anchor": "/tmp/claude-code/.cafleet-base-dir.json"}
# {"status": "unset", "base": null, "source": "absolute-path-arg", "anchor": null}
# {"status": "needs-user-input", "base": null, "source": null, "candidates": ["/tmp/claude-code", "<cwd>"]}

# Record an anchor after AskUserQuestion. Claude calls this after presenting the picker.
cafleet base-dir record --base <abs-path> --source askuserquestion
# Writes <base>/.cafleet-base-dir.json with version=1 and a UTC resolved_at.
# Idempotent: if the anchor already exists and matches, no-op; if it mismatches, error.
```

Both commands:
- Live under the `cafleet` CLI as a new subgroup `base-dir` (alongside `session`, `agent`, `message`, `member`, etc.).
- Do NOT require `--session-id`. Like `cafleet doctor`, they operate on the local filesystem only.
- Default text output is human-readable; `--json` switches to machine-parseable.

`skills/base-dir/SKILL.md` is rewritten to instruct Claude to invoke `cafleet base-dir resolve` first, branch on the `status` field, run `AskUserQuestion` only on `needs-user-input`, and call `cafleet base-dir record` to persist the answer. This shifts the deterministic logic out of skill-markdown-interpreted-by-Claude and into Python — making it unit-testable and immune to Claude-reading-instructions drift.

### 7. Tests (Q9)

**Unit tests** (`cafleet/tests/test_base_dir.py`, new file):

- `test_resolve_absolute_path_argument_returns_unset`
- `test_resolve_cwd_outside_home_returns_cwd`
- `test_resolve_cwd_outside_home_writes_anchor_with_source_cwd_inference`
- `test_resolve_cwd_outside_home_reads_existing_anchor`
- `test_resolve_anchor_mismatch_raises`
- `test_resolve_cwd_under_home_dot_claude_returns_needs_user_input`
- `test_resolve_cwd_under_home_dot_claude_probes_tmp_anchor_first`
- `test_resolve_cwd_equals_home_probes_anchors_in_order`
- `test_record_writes_well_formed_json`
- `test_record_is_idempotent_when_matching`
- `test_record_errors_on_mismatch`
- `test_record_rejects_non_absolute_base`

**CLI tests** (added to `cafleet/tests/test_base_dir.py`):

- `test_cli_resolve_emits_documented_json_shape` — invokes `cafleet base-dir resolve --json` against fixtures exercising each of the four `status` branches in §Specification 6 (`resolved` from cwd-inference, `resolved` from anchor, `unset` from absolute-path-arg, `needs-user-input`). Asserts the JSON shape matches the documented contract field-by-field.
- `test_cli_record_writes_anchor_or_errors_on_mismatch` — invokes `cafleet base-dir record --base <tmp-path> --source askuserquestion`; asserts the anchor lands with the expected JSON, then re-invokes with the same args (idempotent no-op) and with a mismatched base (errors with the documented message and non-zero exit).
- `test_resolve_rejects_unknown_anchor_version` — writes an anchor with `version=2` and `version=0`; asserts `cafleet base-dir resolve` exits with the standardized version-mismatch error and non-zero status for both.

**Integration tests** (`cafleet/tests/test_base_dir_spawn_flow.py`, new file):

- `test_spawn_prompt_substitution_carries_base_to_member` — render every affected spawn-prompt template with `BASE=/some/non-tmp/path`; assert the rendered prompt contains that literal path on a `BASE:` line, positioned immediately after the `YOUR AGENT ID: {agent_id}` line (per §Specification 2 *Insertion position*).
- `test_spawn_prompt_omits_base_line_when_unset` — render every affected spawn-prompt template with `BASE=<unset>`; assert the rendered prompt does NOT contain a `BASE:` line anywhere.
- `test_director_side_audit_writes_land_under_base_not_tmp` — rescoped from the original member-spawning version. Pre-creates an anchor at a pytest-fixture `tmp_path` BASE (a non-`/tmp` directory). Drives the Director-side audit-file write helper directly (no spawned member, no real `cafleet member create`, no real Claude API). Asserts the audit file lands at `<tmp_path>/<role>.md` and no file appears under any `/tmp/` subtree. Real-member end-to-end coverage stays at the manual-smoke level (Implementation Step 4), consistent with §Out of Scope.

The integration tests do not exercise the full design-doc-create flow end-to-end (that would require a real Claude API key); they exercise the spawn-prompt-substitution contract and the Director-side audit-write contract, which are the two new pieces of code this design introduces.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation surfaces (per `.claude/rules/design-doc-numbering.md` — docs FIRST)

`skills/base-dir/SKILL.md` is split into four sub-tasks so PR-time review can verify each rewrite landed independently:

- [x] `skills/base-dir/SKILL.md`: rewrite the Procedure section per §Specification 4 (absolute-path → unset, CWD-deterministic with anchor read/write, AskUserQuestion branch with anchor probe) <!-- completed: 2026-05-12T14:50 -->
- [x] `skills/base-dir/SKILL.md`: add new § *No-bypass write protocol* per §Specification 3 (the 4-item canonical protocol, including the parens-free anchorless `audit-disabled no BASE in spawn prompt` status) <!-- completed: 2026-05-12T14:50 -->
- [x] `skills/base-dir/SKILL.md`: document the `<unset>` sentinel per §Specification 5 (literal string, case-sensitive, guard-then-error contract) <!-- completed: 2026-05-12T14:50 -->
- [x] `skills/base-dir/SKILL.md`: reference the new `cafleet base-dir resolve` / `cafleet base-dir record` CLI subcommands and the gitignore recommendation per §Specification 4 *Gitignore handling* <!-- completed: 2026-05-12T14:50 -->
- [x] Update `skills/create-figure/SKILL.md`: remove the `/tmp/claude-code` override (L24–27 today) per §Specification 1; update the example resolution paragraph and the "never directly in repo root" sentence <!-- completed: 2026-05-12T14:52 -->

For each of the 5 consumer skills below, two sub-tasks: (a) add the `[INSERT BASE]` line in the fixed insertion position to every spawn template the skill owns; (b) replace the inline "Scratch and audit files — never `/tmp`" blurb with a one-line pointer AND update the audit-gating wording (today's wording — e.g., `design-doc-create/SKILL.md:350` — says BASE is "undefined" on the absolute-path branch; rewrite to: "If BASE is the sentinel `<unset>`, the audit-file write is guarded-skipped per `Skill(cafleet:base-dir)` § *No-bypass write protocol* item 1").

- [x] `skills/research-report/SKILL.md`: (a) add `[INSERT BASE]` to Manager/Scout/Researcher spawn templates; (b) blurb pointer replacement + audit-gating wording update <!-- completed: 2026-05-12T14:55 -->
- [x] `skills/research-presentation/SKILL.md`: (a) add `[INSERT BASE]` to Presentation/Transcript/Visual Reviewer spawn templates; (b) blurb pointer replacement + audit-gating wording update <!-- completed: 2026-05-12T14:58 -->
- [x] `skills/design-doc-create/SKILL.md`: (a) add `[INSERT BASE]` to Drafter (normal + resume) and Reviewer spawn templates; (b) blurb pointer replacement + audit-gating wording update at L350 <!-- completed: 2026-05-12T15:00 -->
- [x] `skills/design-doc-execute/SKILL.md`: (a) add `[INSERT BASE]` to Programmer/Tester/Verifier spawn templates; (b) blurb pointer replacement + audit-gating wording update <!-- completed: 2026-05-12T15:02 -->
- [x] `skills/design-doc-interview/SKILL.md`: (a) add `[INSERT BASE]` to Analyzer spawn template; (b) blurb pointer replacement + audit-gating wording update <!-- completed: 2026-05-12T15:03 -->
- [x] Update all 12 *member* role files (Drafter, Reviewer, Manager, Scout, Researcher, Presentation, Transcript, Visual Reviewer, Programmer, Tester, Verifier, Analyzer): add `Skill(cafleet:base-dir)` to the startup-skill load list. Director role files are excluded (already covered by the consumer SKILL.md Step-0 load) <!-- completed: 2026-05-12T15:10 -->
- [x] Update the cafleet repo's own `.gitignore`: add `/.cafleet-base-dir.json` at repo root per §Specification 4 *Gitignore handling* item 1 <!-- completed: 2026-05-12T15:11 -->
- [x] Update `ARCHITECTURE.md`: document the anchor file (location + schema + version policy), the `<unset>` sentinel, the gitignore recommendation, and the new `cafleet base-dir` CLI subgroup <!-- completed: 2026-05-12T15:14 -->
- [x] Update `README.md`: mention the new `cafleet base-dir` CLI subgroup in the CLI reference (use `/update-readme` if scope warrants) <!-- completed: 2026-05-12T15:15 -->

### Step 2: Python helper + CLI

- [x] Add `cafleet/src/cafleet/base_dir.py`: anchor read / write / validate, `resolve()` returning the four status branches, sentinel constant <!-- completed: 2026-05-13T00:35 -->
- [x] Wire `cafleet base-dir resolve` and `cafleet base-dir record` into the click CLI (`cafleet/src/cafleet/cli/`); register the `base-dir` subgroup so `cafleet base-dir --help` works <!-- completed: 2026-05-13T00:40 -->
- [x] `mise //cafleet:lint` + `mise //cafleet:typecheck` + `mise //cafleet:format` clean for the new code <!-- completed: 2026-05-13T00:50 -->

### Step 3: Tests

- [x] Add `cafleet/tests/test_base_dir.py` with all 12 helper unit tests + the 3 CLI tests (incl. `test_resolve_rejects_unknown_anchor_version`) listed in §Specification 7 <!-- completed: 2026-05-13T00:25 -->
- [x] Add `cafleet/tests/test_base_dir_spawn_flow.py` with the 3 integration tests (spawn-prompt substitution carry, omit-on-unset, Director-side audit-write lands under BASE not /tmp) <!-- completed: 2026-05-13T00:25 -->
- [x] `mise //cafleet:test` green <!-- completed: 2026-05-13T00:50; 886 tests pass post-Tester rewire to canonical extract_spawn_templates -->

### Step 4: Validation

- [ ] Manual smoke: run `/design-doc-create` in a non-`/tmp` BASE; verify every spawned role file has `BASE:` in its spawn prompt; verify no files appear under `/tmp/claude-code/` from the member panes <!-- completed: -->
- [ ] Manual smoke: run `/create-figure` from a git-repo CWD; verify figures land at `${CWD}/figures/` and not `/tmp/claude-code/figures/` <!-- completed: -->
- [x] Final removal sweep: grep the repo for any remaining "/tmp/claude-code" reference outside `skills/base-dir/SKILL.md` and `design-docs/0000055-*/`; either justify or delete (per `~/.claude/rules/removal.md` — no historical deprecation notices) <!-- completed: 2026-05-13T01:00; grep sweep clean: remaining refs are intentional (base-dir SKILL.md, create-figure SKILL.md /tmp/claude-code as legitimate BASE choice, base_dir.py _TMP_CANDIDATE, test docstrings, design-doc-0000055 itself); the workflow scratch files at repo root (director-answers.md, drafter*.md) are now gitignored per design 0000055 + the .gitignore reorganization commit -->

---

## Out of Scope

- Data migration of existing `/tmp/claude-code/*` artifacts on user machines (per user Q10 answer — clean break).
- Removal of `/tmp/claude-code (recommended)` from the AskUserQuestion option list (per user Q1 reframing — that option STAYS as long as base-dir is the one selecting it).
- Changes to the `cafleet member create` CLI itself. Spawn-prompt substitution happens Director-side, in skill instructions, before the `cafleet member create` invocation — no broker-side schema or API changes.
- End-to-end real-Claude tests of the design-doc-create / research-report flows. The integration tests cover the prompt-substitution and no-/tmp-writes contracts; they do not invoke a real Claude API.

---

## Changelog

| Date | Changes |
|:--|:--|
| 2026-05-12 | Initial draft |
