# On-Demand Spawn-Prompt Audit + base-dir Migration

**Status**: Complete
**Progress**: 12/13 tasks complete (Step 5 smoke deferred to operator)
**Last Updated**: 2026-05-12

## Overview

Add an on-demand spawn-prompt audit to every CAFleet member spawn, and relocate the `base-dir` skill from the user's global `~/.claude/skills/base-dir/` into the repo-root `skills/base-dir/` so the cafleet plugin becomes self-contained (the migrated copy ships as `cafleet:base-dir` via both `.claude-plugin/plugin.json` and `.codex-plugin/plugin.json`).

The audit feature: after every successful `cafleet member create`, the Director writes the rendered spawn prompt — exactly the text delivered to the member — to a single file under `${BASE}/`. The role definition itself stays in `skills/<skill>/roles/<role>.md`, unchanged from today (role-definition content only, no prologue, no identity block, no spawn-prompt template). The spawn-prompt template (prologue + `<ROLE DEFINITION>...</ROLE DEFINITION>` block + identity block + protocol) stays inline in each consuming skill's `SKILL.md`, also unchanged from today. The Director's *render procedure* gains one extra step at the end: write the assembled, kwarg-substituted prompt to `${BASE}/<role>.md` for operator inspection.

No new `prompts/<role>.md` directory is introduced. The `roles/` directory remains the single on-disk source-of-truth for role-definition content.

## Success Criteria

- [ ] Every successful `cafleet member create` in the five consuming skills (`research-report`, `research-presentation`, `design-doc-create`, `design-doc-execute`, `design-doc-interview`) writes the rendered spawn prompt to `${BASE}/<role>.md` (one file per role-type, overwritten on subsequent spawns of the same role-type within an invocation). <!-- deferred to operator smoke; in-repo instructions are in place (see SC #4) -->
- [x] The `base-dir` skill exists at `skills/base-dir/SKILL.md` and ships as `cafleet:base-dir` via both plugin manifests. <!-- verified: skills/base-dir/SKILL.md tracked; .claude-plugin/plugin.json carries the explicit `./skills/base-dir` entry; .codex-plugin/plugin.json picks it up via the auto-discovery form `"skills": "./skills/"`; both manifests bumped to v0.8.0 -->
- [x] `skills/<skill>/roles/<role>.md` files are unchanged in content from before this design (no flattening, no prologue/identity-block inlining). <!-- verified: `git diff main -- skills/*/roles/*.md` returns empty for the 12 spawned-role files -->
- [x] Each consuming skill's `SKILL.md` Director instructions explicitly call out the audit-file write step after `cafleet member create` returns success. <!-- verified: 12 "Write the audit file" occurrences across 5 SKILL.md files (3+3+2+3+1) -->
- [ ] A `/research-report` smoke run end-to-end succeeds with the new audit-file write and produces one rendered file under `${BASE}/` per spawned member-role. <!-- deferred to operator post-merge smoke -->


---

## Background

CAFleet member spawn prompts are assembled by the Director from two sources:

1. **`SKILL.md`** carries the spawn-prompt template inline (prologue + `[ROLE DEFINITION]` placeholder + identity block + protocol + trailing instructions) and the three cafleet kwargs `{session_id}` / `{agent_id}` / `{director_agent_id}`.
2. **`roles/<role>.md`** carries the role definition; the Director substitutes this verbatim into the `[ROLE DEFINITION]` placeholder at render time.

Two pain points:

- **Audit gap.** After `cafleet member create` succeeds, there is no on-disk record of the fully rendered prompt that was actually passed to the new member. Operators inspecting a failed run cannot tell which `[INSERT ...]` values the Director substituted, which version of the role doc was injected, or whether the prompt was malformed.
- **Plugin self-containment gap.** `base-dir` lives only at `~/.claude/skills/base-dir/`, so operators who install the cafleet plugin without the global copy see `Skill(base-dir)` fail to resolve, and every consuming skill's Step 0 breaks. The cafleet plugin should ship its own copy.

The two changes are bundled in one design doc because they share a single migration commit boundary — the audit-file write depends on `Skill(cafleet:base-dir)` resolving `BASE`.

---

## Specification

### Coverage

In scope: the five base-dir-consuming skills that spawn members.

| Skill | Skills dir | Spawned members | Role files (unchanged) |
|:--|:--|:--|:--|
| `research-report` | `skills/research-report/` | Manager, Scout, Researcher | `roles/manager.md`, `roles/scout.md`, `roles/researcher.md` |
| `research-presentation` | `skills/research-presentation/` | Presentation, Transcript, Visual Reviewer | `roles/presentation.md`, `roles/transcript.md`, `roles/visual-reviewer.md` |
| `design-doc-create` | `skills/design-doc-create/` | Drafter (two modes: normal, resume), Reviewer | `roles/drafter.md`, `roles/reviewer.md` |
| `design-doc-execute` | `skills/design-doc-execute/` | Programmer, Tester, Verifier | `roles/programmer.md`, `roles/tester.md`, `roles/verifier.md` |
| `design-doc-interview` | `skills/design-doc-interview/` | Analyzer | `roles/analyzer.md` |

Out of scope for the audit-file write: `agent-team-monitoring`, `agent-team-supervision`, `my-slidev`, `create-figure`, `cafleet`. These skills do not spawn CAFleet members and therefore do not participate in the audit-file write described here.

**`create-figure` exception (in-scope for the namespace rename):** `create-figure` does load `Skill(base-dir)` for its own output-directory resolution. To keep `/create-figure` working after the operator removes `~/.claude/skills/base-dir/`, the `Skill(base-dir)` invocation in `skills/create-figure/SKILL.md` MUST be updated to `Skill(cafleet:base-dir)` as part of Step 4 (treated as the sixth consuming skill for that step's namespace-rename portion; create-figure does NOT get an audit-file write step because it spawns no members).

Non-spawned `roles/director.md` files document the main-Claude Director's behavior. They remain in `roles/` as reference documentation.

### On-Disk Layout (unchanged)

```
skills/<skill>/
├── SKILL.md          # inline spawn-prompt template + Director instructions
└── roles/
    ├── <role-1>.md   # role definition only (no prologue, no identity block)
    ├── <role-2>.md
    └── director.md   # reference documentation, not a spawn-prompt
```

This is the current layout. The design does NOT introduce a new `prompts/` directory and does NOT flatten the role files. The only on-disk change is the new file at `${BASE}/<role>.md` that the Director writes per spawn.

### Three-Tier Variable Substitution Model

The existing three-tier model is preserved. Every spawn-prompt template (inline in `SKILL.md`) mixes three kinds of placeholders, resolved in this order:

| Tier | Marker syntax | Substituted by | When | Example |
|:--|:--|:--|:--|:--|
| 1. Skill-side fill-in | `[INSERT <description>]` | The Director, **before** the `member create` call | Per-spawn (skill-specific values) | `[INSERT today's date]`, `[INSERT output directory]`, `[INSERT USER'S ORIGINAL REQUEST]` |
| 2. Verbatim role inlining | `[ROLE DEFINITION]` placeholder, replaced with the verbatim content of `roles/<role>.md` | The Director, **before** the `member create` call | Per-spawn | n/a after substitution |
| 3. cafleet kwargs | `{session_id}`, `{agent_id}`, `{director_agent_id}` | `cafleet member create` via Python `str.format()` on the entire prompt string | At spawn time | Single-braced; never doubled |

The tier-1 marker syntax is a **notational convention** — `[INSERT <description>]` means "substitute the value the consuming skill computed earlier in its procedure." The Director performs the substitution in-memory before passing the rendered prompt to `cafleet member create`; no shell variable expansion is involved.

Authoring rule for `SKILL.md` spawn-prompt blocks and `roles/<role>.md` files: tier-3 single-braced kwargs are the **only** literal `{` / `}` characters permitted in the assembled prompt. Any other `{` or `}` (e.g., a JSON example inside a role definition) MUST be doubled to `{{` / `}}` before being read by the Director — `str.format()` raises `KeyError` at spawn time otherwise. Implementation **Step 3** runs a one-shot brace audit over every inline `SKILL.md` spawn-prompt block and every `roles/<role>.md` file, doubles any stray unescaped `{` / `}`, and only then permits the Step 4 audit-file write step to land — so the migration does not surface a latent `KeyError` at the first spawn after merge.

### Director Render Procedure

For each `cafleet member create`, the Director's flow becomes:

1. **Read** the inline spawn-prompt template from this consuming skill's `SKILL.md` (the fenced block under the per-role spawn section).
2. **Read** `skills/<skill>/roles/<role>.md` (the role definition).
3. **Substitute `[ROLE DEFINITION]`** in the template with the verbatim content of step 2's read.
4. **Substitute every `[INSERT ...]` tier-1 marker** with the corresponding skill-side value (string replace, in memory). The Director MUST resolve every `[INSERT ...]` marker; any unresolved marker is a bug, not a fallthrough.
5. **Pass the result** as the positional spawn-prompt argument to `cafleet --json member create --agent-id <director-agent-id> -- "<rendered prompt>"`. cafleet's own `_resolve_prompt` performs tier-3 `str.format()` on this string with the four kwargs.
6. **Parse `agent_id`** from the `member create` JSON response. Treat it as the new member's literal UUID for all subsequent `cafleet` calls.
7. **Re-render the final prompt locally** with Python `str.format()` semantics over the rendered string and the three kwargs — i.e., conceptually `prompt.format(session_id=<session-id>, agent_id=<new-member-id>, director_agent_id=<director-agent-id>)`. This mirrors cafleet's broker-side substitution and yields the **exact** text the new member sees in its pane.
8. **Write the audit file** to `${BASE}/<role>.md` (see § *Audit File Layout* below).

Steps 1–6 are today's procedure unchanged. Steps 7–8 are new.

### Audit File Layout

After every successful `cafleet member create`, the Director writes:

```
${BASE}/<role>.md
```

| Component | Source | Notes |
|:--|:--|:--|
| `${BASE}` | The `BASE` value resolved by `Skill(cafleet:base-dir)` in the consuming skill's Step 0 | The audit feature is **gated on Step 0 having completed**; nothing is written if `BASE` is not yet resolved. |
| `<role>` | The role-type slug (kebab-case) — `manager`, `scout`, `researcher`, `presentation`, `transcript`, `visual-reviewer`, `drafter`, `drafter-resume`, `reviewer`, `programmer`, `tester`, `verifier`, `analyzer` | Independent of the cafleet member `--name` (which may carry an additional ordinal like `vr-batch-7`). The audit filename uses the **role type**, not the member name. |

File contents: the **fully rendered prompt only** — exactly the text the member sees in its pane after tier-1, tier-2, and tier-3 substitutions. No YAML frontmatter, no sidecar JSON, no metadata header. Operators recover metadata (session id, member agent id, timestamp, originating command) from the broker timeline / admin WebUI.

#### Overwrite Semantics

`${BASE}/<role>.md` is overwritten in place on subsequent spawns of the **same role-type** within the same skill invocation. There is no slot ordinal (`-1`, `-2`, etc.) — the latest spawn wins. Rationale: keeps the audit folder a flat, predictable workspace; per-spawn forensics live in the broker timeline (canonical) and in the immediate-prior file content (if the operator captures it before the overwrite). Operators who need per-spawn isolation can run each invocation against a distinct `BASE` (`Skill(cafleet:base-dir)` supports this).

A re-spawn after a crash within the same invocation also overwrites the audit file with the new rendered prompt. Pre-existing audit files from a prior invocation are likewise overwritten in place without warning.

### Write Timing

The Director writes the audit file **after** `cafleet member create` returns success (exit 0 with a parseable `agent_id`). A failed `member create` leaves no audit file on disk; spawn-failure forensics live in the Director's pane buffer and the broker timeline, not on disk.

Rationale: writing post-success keeps the audit log a ground-truth record of what was *delivered*, not what was *attempted*.

### `base-dir` Skill Migration

(Already implemented per Step 2; restated here for reference.)

| Aspect | Decision |
|:--|:--|
| **Source path** | `~/.claude/skills/base-dir/SKILL.md` (19 lines). |
| **Destination path** | `skills/base-dir/SKILL.md` (repo-root plugin source; ships as `cafleet:base-dir` via both `.claude-plugin/plugin.json` and `.codex-plugin/plugin.json`). |
| **Migration mode** | **Move only** — the plugin-shipped copy is the sole copy after migration. |
| **Skills directory choice** | `skills/` (repo-root plugin source). Ship base-dir as part of the cafleet plugin so it auto-installs alongside the consuming skills. Consuming skills invoke `Skill(cafleet:base-dir)` (prefixed) post-migration. |
| **Content** | Identical to the current global file. No procedural changes. |
| **Global-copy removal** | The user manually deletes `~/.claude/skills/base-dir/` **after** the migration commit lands and the Step 5 smoke-test passes. The design doc cannot delete files outside the cafleet repo; this is documented as a follow-up operator step. |

Consuming skills MUST invoke `Skill(cafleet:base-dir)` (prefixed) post-migration. The plugin loader registers the skill under the `cafleet:` namespace; the bare-name `Skill(base-dir)` form will not resolve once the global copy at `~/.claude/skills/base-dir/` is removed. Every base-dir consumer must be updated as part of Step 4 to use the prefixed form.

### Failure-Mode Reasoning

| Scenario | Behavior |
|:--|:--|
| `Skill(cafleet:base-dir)` fails to load post-migration (e.g., the user has neither the global nor the repo copy installed). | Step 0 of every consuming skill aborts immediately — same failure mode as today, with the global copy missing. The audit-file write is unreachable because `BASE` was never resolved. |
| `${BASE}/<role>.md` write fails (e.g., disk full, permission denied). | The Director logs a one-line warning and proceeds without rolling back the spawn — the member is already live in its pane. Degraded mode: subsequent spawns may also fail to write. The Director does NOT escalate via `AskUserQuestion` for an isolated write failure; only if multiple consecutive writes fail does the Director surface the issue. |
| A `roles/<role>.md` file is missing at render time. | Hard error — the Director aborts the spawn for that role with a clear message ("`roles/<role>.md` not found in skills/<skill>/"). This is a regression bug in the skill, not a runtime fallback. |

### Coordination Protocol Compatibility

The design doc does NOT introduce new verbs, new pointer forms, or new `COMMENT(role)` markers. The five consuming skills' existing coordination protocols (verb + pointer schema in `/design-doc-create` and `/design-doc-execute`; free-form clarification-phase exemptions) are untouched. Audit-file writes are a Director-side side effect with no on-broker representation — they do not appear in the timeline, and the design doc itself never references them in a `cafleet message send` body.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation Audit & Top-Level References

`base-dir` is an internal skill — it is loaded by consuming skills via `Skill(cafleet:base-dir)` (prefixed) and is explicitly NOT user-invocable. It therefore does NOT belong in the "Project Skills" lists in `CLAUDE.md` or `.claude/CLAUDE.md`, which enumerate user-invocable slash commands only. Claude Code's system-reminder surfaces `cafleet:base-dir` to teammates that need it without any project-manifest entry.

- [x] Grep `README.md` (and `ARCHITECTURE.md` and `docs/` if present) for the strings `base-dir`, `~/.claude/skills/base-dir`, and `Skill(base-dir)`. Update any path reference to point at the new repo-root plugin source `skills/base-dir/`. Update any `Skill(base-dir)` invocation to the prefixed `Skill(cafleet:base-dir)` form. If nothing is found, the task is no-op and gets checked off as such. <!-- completed: 2026-05-11T12:45 -->
- [x] Verify `CLAUDE.md` and `.claude/CLAUDE.md` do NOT add a `/base-dir` entry to their Project Skills sections. (The skill is non-user-invocable; the system-reminder is its sole discovery path.) <!-- completed: 2026-05-11T12:45 -->

### Step 2: Migrate `base-dir` Skill

- [x] Create `skills/base-dir/SKILL.md` with the exact 19-line content of `~/.claude/skills/base-dir/SKILL.md`. <!-- completed: 2026-05-11T13:15 -->
- [x] Edit `.claude-plugin/plugin.json`: append `"./skills/base-dir"` to the `skills` array. Bump `version` per the semver policy in design 0000054 (adding a shipped skill is a MINOR bump). Edit `.codex-plugin/plugin.json`: bump `version` to match; the `"skills": "./skills/"` auto-discovery field is unchanged. Edit `cafleet/pyproject.toml`: bump the version to match the plugin manifests. <!-- completed: 2026-05-11T13:15; 0.7.0 → 0.8.0; also added base-dir to mise.toml sync-skills task -->
- [x] Run `mise //:sync-skills` so the home-directory working copy at `~/.claude/skills/base-dir/` is synchronized from the new plugin-source copy. (Design 0000054 acknowledges the home-directory working copy is operator-managed; this sync keeps the maintainer's local environment consistent.) <!-- completed: 2026-05-11T13:15; dispatched via Director member exec -->
- [x] Verify the new file is picked up by Claude Code skill discovery: open a fresh Claude Code session in the cafleet repo and confirm `cafleet:base-dir` (prefixed) appears in the system-reminder skill list. (Manual smoke step before Step 3 proceeds.) <!-- completed: 2026-05-11T13:15; system-reminder refreshed mid-session and now lists both `base-dir` and `cafleet:base-dir` -->

### Step 3: Brace Audit

- [x] For each of the five consuming skills, grep `SKILL.md` and every `roles/*.md` file under that skill for literal `{` and `}` characters. Every occurrence outside the three cafleet kwargs (`{session_id}`, `{agent_id}`, `{director_agent_id}`) MUST already be doubled (`{{` / `}}`). Fix any unescaped occurrence in-place before Step 4 proceeds. <!-- completed: 2026-05-11T13:20; audit clean — role files have zero braces; SKILL.md spawn-prompt blocks contain only the three kwargs; `${VAR}` references in narrative + `[INSERT ${VAR}]` markers are out-of-scope (tier-1 substituted by Director before str.format) -->

### Step 4: Update SKILL.md Director Instructions (Audit-File Write)

For each consuming skill, edit `SKILL.md`'s per-role spawn section to add two new steps after `cafleet member create` parsing: (a) re-render the prompt locally with `prompt.format(session_id=..., agent_id=<new-member-id>, director_agent_id=...)` (in memory; the Director can shell out via `python -c` or do the substitution textually — implementation choice per skill), and (b) Write the result to `${BASE}/<role>.md`. Also update every `Skill(base-dir)` invocation in the affected SKILL.md to the prefixed `Skill(cafleet:base-dir)` form — bare-name resolution does not work post-migration once the global `~/.claude/skills/base-dir/` copy is removed.

Reference template for the added steps (adapt per skill — outer fence is four backticks so the inner ```bash fence renders correctly):

````
After parsing `agent_id` from the `member create` JSON response, append:

1. **Re-render the prompt locally** with the three kwargs bound: `session_id` = `<session-id>`, `agent_id` = the parsed `<new-member-id>`, `director_agent_id` = `<director-agent-id>`. The result equals the exact text the new member sees in its pane.
2. **Write the audit file** to `${BASE}/<role>.md` (`${BASE}` resolved by `Skill(cafleet:base-dir)` in Step 0; `<role>` is the role-type slug, e.g. `manager`, `programmer`). Overwrites on subsequent spawns of the same role-type within this invocation; that is intentional.
````

- [x] Update `skills/research-report/SKILL.md`: add the audit-file write step (5+6) to each of the three per-role spawn sections (Manager, Scout, Researcher). Update `Skill(base-dir)` invocations to `Skill(cafleet:base-dir)`. <!-- completed: 2026-05-11T13:51 -->
- [x] Update `skills/research-presentation/SKILL.md`: same for Presentation, Transcript, Visual Reviewer. Update `Skill(base-dir)` invocations to `Skill(cafleet:base-dir)`. <!-- completed: 2026-05-11T13:51 -->
- [x] Update `skills/design-doc-create/SKILL.md`: same for Drafter (both normal and resume modes share the same Director procedure location) and Reviewer. Update `Skill(base-dir)` invocations to `Skill(cafleet:base-dir)`. <!-- completed: 2026-05-11T13:51; drafter step covers both normal and resume modes; audit filename branches drafter.md / drafter-resume.md -->
- [x] Update `skills/design-doc-execute/SKILL.md`: same for Programmer, Tester, Verifier. Update `Skill(base-dir)` invocations to `Skill(cafleet:base-dir)`. <!-- completed: 2026-05-11T13:51 -->
- [x] Update `skills/design-doc-interview/SKILL.md`: same for the single Analyzer spawn section. Update `Skill(base-dir)` invocations to `Skill(cafleet:base-dir)`. <!-- completed: 2026-05-11T13:51 -->

### Step 5: Smoke-Test the Migration

- [ ] Run `/research-report` with a short topic phrase (e.g., `"tcp-vs-udp"`) end-to-end in a fresh tmux session inside a fresh Claude Code instance that has the cafleet plugin installed. Confirm: (i) `Skill(cafleet:base-dir)` resolves correctly from the plugin location, (ii) every spawned member starts successfully, (iii) `${BASE}/<role>.md` files are written for every spawned role (`${BASE}/manager.md`, `${BASE}/scout.md`, `${BASE}/researcher.md` — the last one overwritten if 3 Researchers spawned), (iv) each audit-file's content matches the prompt the corresponding pane received (verify via `cafleet member capture` for one pane and diffing against the audit file). <!-- deferred to operator: the smoke test requires a fresh Claude Code session in a fresh tmux session and cannot be executed from the implementation team's pane; the in-repo deliverables that make the smoke runnable are verified via SC #2/#3/#4 -->

**Operator follow-up (outside this design doc's authority — no checkbox).** After Step 5 passes, the operator manually removes `~/.claude/skills/base-dir/` so the repo-root plugin source `skills/base-dir/` (installed as `cafleet:base-dir`) is the sole copy on disk. This action lives outside the cafleet repo and cannot be performed by the implementation team; the in-repo deliverable (`Success Criterion #2`) is satisfied independently of when the operator runs the removal.

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-11 | Initial draft |
| 2026-05-11 | Round-1 review: promoted brace audit to Step 3; relocated base-dir manifest tasks (skill is non-user-invocable); fixed nested-fence rendering bug; moved global-skill removal into operator-follow-up paragraph. |
| 2026-05-11 | Resume-mode revision: applied interview Q1-Q12 answers; dropped audit-file feature scope; rewrote base-dir migration target to skills/base-dir/ plugin destination; updated Coverage table and Step 4 paths to post-0054 layout. |
| 2026-05-11 | Round-2 review: reverted Status to Draft pending Reviewer approval; trimmed SC #2 to the in-repo deliverable only (global-copy removal lives solely in the Step 6 operator-follow-up paragraph); enumerated post-migration roles/director.md survivors per skill and added Step 4e clean-up sub-task to delete the empty design-doc-interview/roles/ directory; clarified that the `<ROLE DEFINITION>` tag pair is preserved around the inlined content; dropped sub-clause (iv) from the Step 6 smoke test (no need to verify absence of a never-implemented feature); trimmed the Round-1 changelog row to surviving items; task count 0/26 → 0/27 for the new Step 4e clean-up task. |
| 2026-05-11 | User approval — Status promoted from Draft to Approved; ready for `/design-doc-execute`. |
| 2026-05-11 | Doc reconstructed from conversation transcript after an in-session `git checkout` clobbered uncommitted revision changes. Content matched the user-approved Round-2 state. |
| 2026-05-11 | Step 1 (no-op audit) and Step 2 (base-dir migration to skills/base-dir/, version bump 0.7.0→0.8.0, mise sync-skills extension) and Step 3 (brace audit clean) executed and committed. |
| 2026-05-11 | Mid-implementation pivot: user determined that the `prompts/<role>.md` externalization approach was not the intent. Design revised to drop the `prompts/` directory entirely. `roles/<role>.md` stays unchanged (role definitions only, as today). Spawn-prompt template stays inline in `SKILL.md` (as today). The audit-file feature is restored (reverses interview Q11) with a simpler shape: rendered prompt written to `${BASE}/<role>.md` per role-type, overwritten on subsequent spawns. Status reverted to Draft pending revision review. Title renamed from "Spawn-Prompt Files and base-dir Migration" to "On-Demand Spawn-Prompt Audit + base-dir Migration". Task count revised to 13 (Steps 1+2+3 already done = 7/13). Step 4 was previously the prompts/ flatten across 5 skills; it is now the SKILL.md Director-instruction update to add the audit-file write step. Step 5 is the smoke test (was Step 6). Step 6 is removed. Programmer's mid-Step-4 work-in-progress (13 prompts/<role>.md files) was deleted and the 12 deleted role files restored from git HEAD before this revision was written. |
| 2026-05-11 | Post-pivot Status promoted: Draft → Approved upon user re-approval (the user typed "commit and go" after reviewing the revised design; the implementation team then completed the revised Step 4 and pushed PR #66). Copilot review of PR #66 is in progress. Status will be promoted to Complete by the orchestrating `/design-doc-execute` skill's finalize phase after the Copilot review loop exits — that phase is external to this design's own Implementation steps (which end at Step 5). |
| 2026-05-12 | Status: Approved → Complete. PR #66 merged into main and all Copilot review fixes landed; the in-repo deliverables (SC #2, #3, #4) are verified, and SC #1 / #5 remain operator-deferred per the Step 5 follow-up paragraph. Finalize executed in local-only mode (already on main; no push or PR needed). |
