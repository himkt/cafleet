# Spawn-Prompt Files and base-dir Migration

**Status**: Approved
**Progress**: 0/24 tasks complete
**Last Updated**: 2026-05-11

## Overview

Externalize CAFleet member spawn prompts from inline `SKILL.md` blocks into per-role `prompts/<role>.md` files, write each rendered (final) prompt to `<base-dir>/spawn-prompts/<role>-<n>.md` after a successful `cafleet member create` for operator audit visibility, and relocate the `base-dir` skill from the user's global `~/.claude/skills/base-dir/` into the repo-local `cafleet/.claude/skills/base-dir/` so the cafleet plugin becomes self-contained.

## Success Criteria

- [ ] Every base-dir-consuming skill (`research-report`, `research-presentation`, `design-doc-create`, `design-doc-execute`, `design-doc-interview`) reads its member spawn prompts from `prompts/<role>.md` files instead of `SKILL.md`-inline blocks.
- [ ] Every successful `cafleet member create` in those skills produces a rendered audit file at `<base-dir>/spawn-prompts/<role>-<n>.md` containing the exact prompt text that was passed to `member create`.
- [ ] The `base-dir` skill exists at `cafleet/.claude/skills/base-dir/SKILL.md`; the global copy at `~/.claude/skills/base-dir/` is removed by the operator after the migration merges.
- [ ] Existing `roles/<role>.md` files in the five consuming skills are flattened into the corresponding `prompts/<role>.md` files and deleted; `SKILL.md` references to `roles/...` are updated to point at `prompts/...`.
- [ ] A `/research-report` smoke run end-to-end succeeds with the new layout and produces audit files for every spawned member.

---

## Background

Today, CAFleet member spawn prompts live in two places per skill:

1. **`SKILL.md`** carries the spawn-prompt template inline (e.g., a fenced block titled "Manager spawn prompt:" containing `[ROLE DEFINITION]` placeholders, `[INSERT ...]` markers, and the four cafleet kwargs `{session_id}` / `{agent_id}` / `{director_name}` / `{director_agent_id}`).
2. **`roles/<role>.md`** carries the role definition that the Director injects verbatim into the `[ROLE DEFINITION]` block at assembly time.

Two pain points:

- **Audit gap.** After `cafleet member create` succeeds, there is no on-disk record of the fully rendered prompt that was actually passed to the new member. Operators inspecting a failed run cannot tell which `[INSERT ...]` values the Director substituted, which version of the role doc was injected, or whether the prompt was malformed.
- **Plugin self-containment gap.** The `base-dir` skill (which resolves `BASE` for every consuming skill's output directory) lives only in the user's global `~/.claude/skills/base-dir/`. Operators who install the cafleet plugin without the global skill see `Skill(base-dir)` fail to resolve, and every consuming skill's Step 0 breaks. The cafleet plugin should ship its own copy.

The two changes are bundled in one design doc because they touch the same files (every consuming-skill `SKILL.md`) and share a single migration commit boundary.

---

## Specification

### Coverage

In scope: the five base-dir-consuming skills that spawn members.

| Skill | Skills dir | Spawned members | Existing `roles/<role>.md` files (to flatten) |
|:--|:--|:--|:--|
| `research-report` | `.claude/skills/research-report/` | Manager, Scout, Researcher | `roles/manager.md`, `roles/scout.md`, `roles/researcher.md` |
| `research-presentation` | `.claude/skills/research-presentation/` | Presentation, Transcript, Visual Reviewer | `roles/presentation.md`, `roles/transcript.md`, `roles/visual-reviewer.md` |
| `design-doc-create` | `skills/design-doc-create/` | Drafter (two modes: normal, resume), Reviewer | `roles/drafter.md`, `roles/reviewer.md` |
| `design-doc-execute` | `skills/design-doc-execute/` | Programmer, Tester, Verifier | `roles/programmer.md`, `roles/tester.md`, `roles/verifier.md` |
| `design-doc-interview` | `skills/design-doc-interview/` | Analyzer | `roles/analyzer.md` |

Out of scope (do NOT introduce a `prompts/` folder, do NOT consume the new audit log): `agent-team-monitoring`, `agent-team-supervision`, `my-slidev`, `create-figure`, `cafleet`. These skills are not base-dir consumers.

Non-spawned `roles/director.md` files (e.g., `research-report/roles/director.md`, `research-presentation/roles/director.md`, `design-doc-create/roles/director.md`) document the main-Claude Director's behavior. They are NOT spawn-prompt templates and are NOT migrated into `prompts/`. They remain in `roles/` as reference documentation and are linked from `SKILL.md` unchanged.

### Template File Layout

```
skills/<skill>/
├── SKILL.md
├── prompts/                 # NEW
│   ├── <role-1>.md
│   ├── <role-2>.md
│   └── ...
└── roles/                   # post-migration: ONLY non-spawned roles (e.g., director.md)
    └── director.md          # unchanged
```

Each `prompts/<role>.md` is a **single self-contained spawn-prompt template** that carries:

1. The prologue line ("You are the X in a Y team (CAFleet-native)."), unchanged from today's inline `SKILL.md` block.
2. A `<ROLE DEFINITION>...</ROLE DEFINITION>` block with the role-definition content **inlined verbatim** (flattened from the old `roles/<role>.md` file).
3. The "Load these skills at startup:" list, unchanged.
4. The literal-UUID identity block (`SESSION ID: {session_id}`, `DIRECTOR AGENT ID: {director_agent_id}`, `YOUR AGENT ID: {agent_id}`, etc.).
5. Skill-side `[INSERT ...]` markers for values the Director substitutes per spawn (USER REQUEST, OUTPUT DIRECTORY, CURRENT DATE, DESIGN DOCUMENT, YOUR ASSIGNMENT, YOUR TASK ID, etc.).
6. The communication protocol block, unchanged.
7. Any trailing instructions, unchanged.

### Three-Tier Variable Substitution Model

The existing three-tier model is preserved verbatim. Every `prompts/<role>.md` file mixes three kinds of placeholders, resolved in this order:

| Tier | Marker syntax | Substituted by | When | Example |
|:--|:--|:--|:--|:--|
| 1. Skill-side fill-in | `[INSERT <description>]` or `[INSERT ${VAR}]` | The Director, **before** the `member create` call | Per-spawn (skill-specific values) | `[INSERT today's date]`, `[INSERT ${OUTPUT_DIR}]`, `[INSERT USER'S ORIGINAL REQUEST]` |
| 2. Verbatim role inlining | `<ROLE DEFINITION>...content...</ROLE DEFINITION>` | (none — content is **already inlined** post-flatten; the tag pair stays as a visual delimiter) | Authoring time (one-shot at migration) | n/a after flatten |
| 3. cafleet kwargs | `{session_id}`, `{agent_id}`, `{director_name}`, `{director_agent_id}` | `cafleet member create` via Python `str.format()` on the entire prompt string | At spawn time | Single-braced; never doubled |

Authoring rule for `prompts/<role>.md` files: tier-3 single-braced kwargs are the **only** literal `{` / `}` characters permitted in the file. Any other `{` or `}` (e.g., a JSON example inside the role definition) MUST be doubled to `{{` / `}}` before being written into the file — `str.format()` raises `KeyError` at spawn time otherwise. This is the same rule the existing inline `SKILL.md` blocks already enforce in principle, but it has not been audited across the role docs that are about to be flattened. Implementation **Step 3** runs a one-shot brace audit over every inline `SKILL.md` spawn-prompt block and every `roles/<role>.md` file, doubles any stray unescaped `{` / `}`, and only then permits the Step 4 flatten — so the migration does not surface a latent `KeyError` at the first spawn after merge.

### Director Render Procedure

After the migration, the Director's spawn flow becomes:

1. **Read** `prompts/<role>.md` from disk (`Read` tool) — the path is relative to the skill directory.
2. **Substitute tier-1 `[INSERT ...]` markers** with skill-side values (string replace, in memory). The Director MUST resolve every `[INSERT ...]` marker; any unresolved marker is a bug, not a fallthrough.
3. **Pass the result** as the positional spawn-prompt argument to `cafleet --json member create --agent-id <director-agent-id> -- "<rendered prompt>"`. cafleet's own `_resolve_prompt` performs tier-3 `str.format()` on this string with the four kwargs.
4. **Parse `agent_id`** from the `member create` JSON response. Treat it as the new member's literal UUID for all subsequent `cafleet` calls.
5. **Re-render the final prompt locally** by running Python `str.format()` semantics (or the equivalent) over the rendered string with the four kwargs — i.e., conceptually `prompt.format(session_id=<session-id>, agent_id=<new-member-id>, director_name=<director-name>, director_agent_id=<director-agent-id>)`. This mirrors cafleet's own broker-side `_resolve_prompt` substitution: doubled `{{` / `}}` are un-escaped to single `{` / `}`, single-braced kwargs are replaced, and the result equals the **exact** text the new member sees in its pane. A naïve `str.replace("{session_id}", …)` is NOT acceptable — it leaves doubled braces untouched and diverges from the member's view.
6. **Write the audit file** (see § *Audit File Layout* below).

The re-rendering step in (5) duplicates cafleet's own `str.format()` substitution, but that is intentional: the audit file is meant to capture what was *delivered to the member*, not what was *passed to the broker*. The two only differ in the four kwargs.

### Audit File Layout

After every successful `cafleet member create`, the Director writes:

```
<base-dir>/spawn-prompts/<role>-<n>.md
```

| Component | Source | Notes |
|:--|:--|:--|
| `<base-dir>` | The `BASE` value resolved by `Skill(base-dir)` in the consuming skill's Step 0 | The audit feature is **gated on Step 0 having completed**; nothing is written if `BASE` is not yet resolved. |
| `spawn-prompts/` | Literal subdirectory name | Created on first write of each invocation if missing. |
| `<role>` | The role-type slug (kebab-case) — `manager`, `scout`, `researcher`, `presentation`, `transcript`, `visual-reviewer`, `drafter`, `drafter-resume`, `reviewer`, `programmer`, `tester`, `verifier`, `analyzer` | Independent of the cafleet member `--name` (which may carry an additional ordinal like `vr-batch-7`). The audit filename uses the **role type**, not the member name. |
| `<n>` | A **slot ordinal** owned by the Director, scoped to **this single skill invocation**. The Director maintains an in-memory `{ role → next_slot }` map for the lifetime of the invocation; the first spawn of a role takes slot `1`, the second takes slot `2`, etc. A re-spawn for crash recovery reuses the **same slot** that the crashed member held, overwriting the existing audit file. | Resets per invocation. Singletons still get a `-1` suffix (e.g., `manager-1.md`, `drafter-1.md`). The Director does NOT derive the counter by counting on-disk files or by querying `cafleet member list`; the in-memory map is authoritative. |

#### Counter Source of Truth

The Director keeps the slot map in its own working memory (the running main-Claude task context) for the duration of the skill invocation. The map is NOT persisted to disk and is NOT recoverable across Director restarts — a fresh `/research-report` invocation starts with an empty map even if `<base-dir>/spawn-prompts/` already contains files from a prior run. This keeps the assignment of slots deterministic with respect to the in-flight invocation and free from drift caused by stale on-disk files. The map is implicit (no file is written), so the Director simply tracks "I have spawned N researchers so far" while orchestrating, exactly as it already tracks `<researcher-NN-agent-id>` values today.

File contents: the **rendered prompt only** — exactly the text the member sees in its pane. No YAML frontmatter, no sidecar JSON, no metadata header. Operators recover metadata (session id, member agent id, timestamp, originating command) from the broker timeline / admin WebUI.

#### Stale Files from Prior Invocations

The Director does NOT wipe `<base-dir>/spawn-prompts/` at the start of an invocation, nor does it scope writes to a per-invocation subfolder. The audit folder is treated as a flat, mutable, overwrite-in-place workspace (see Q12). Consequence: if invocation 1 produced `researcher-1.md`, `researcher-2.md`, `researcher-3.md` and invocation 2 only spawns 2 Researchers, the leftover `researcher-3.md` from invocation 1 silently persists alongside the freshly written `researcher-1.md` / `researcher-2.md` from invocation 2, with no on-disk indication of which run produced it. This is **accepted staleness** (option (c) — operator responsibility), justified by: (i) the broker timeline / admin WebUI is the canonical audit trail; the on-disk files are a convenience surface, (ii) the operator can `rm -rf` the folder between invocations if a clean snapshot is required, and (iii) `git status` will show drift if the folder is tracked. Operators who need run-by-run isolation can run each invocation against a distinct `BASE` (Skill(base-dir) supports this) instead of overlaying runs in the same folder.

Examples:

| Skill invocation | Audit files produced |
|:--|:--|
| `/research-report "topic X"` spawning 1 Manager, 1 Scout, 3 Researchers | `manager-1.md`, `scout-1.md`, `researcher-1.md`, `researcher-2.md`, `researcher-3.md` |
| `/research-presentation foo` spawning Presentation, Transcript, and 2 VR batches | `presentation-1.md`, `transcript-1.md`, `visual-reviewer-1.md`, `visual-reviewer-2.md` |
| `/design-doc-create my-feature` in normal mode | `drafter-1.md`, `reviewer-1.md` |
| `/design-doc-create my-feature` in resume mode (post-interview) | `drafter-resume-1.md`, `reviewer-1.md` |
| `/design-doc-execute my-feature` spawning Programmer, Tester, and Verifier | `programmer-1.md`, `tester-1.md`, `verifier-1.md` |
| `/design-doc-interview my-feature` | `analyzer-1.md` |

### Write Timing

The Director writes the audit file **after** `cafleet member create` returns success (exit 0 with a parseable `agent_id`). A failed `member create` leaves no audit file on disk; spawn-failure forensics live in the Director's pane buffer and the broker timeline, not on disk.

Rationale: writing post-success keeps the audit log a ground-truth record of what was *delivered*, not what was *attempted*. The handful of attempted-but-failed spawns per session would clutter the audit folder with prompts that no member ever saw.

### Re-spawn and Overwrite Semantics

| Situation | Audit-file behavior |
|:--|:--|
| **First spawn of a role in this invocation** | Write `<role>-1.md`. |
| **Subsequent spawns of the same role in this invocation** (e.g., a Manager requests a 4th Researcher mid-run) | Increment the counter: write `<role>-4.md`. The counter is owned by the Director and reset to 1 on every new skill invocation. |
| **Re-spawn after a crash** within the same invocation (e.g., `researcher-2` dies, Director re-spawns with a new agent id under the same role+ordinal) | **Overwrite** `researcher-2.md` with the new rendered prompt. Latest spawn wins. Pre-crash forensics live in the broker timeline (per the cafleet recovery decision tree). |
| **Pre-existing `<base-dir>/spawn-prompts/` from a prior skill invocation** | **Overwrite in place** on a per-file basis. No `.bak` rename, no timestamp subfolder, no preflight error. The audit folder is mutable; prior-run audit content is recoverable via `git diff` if the operator has committed it, or is simply lost. |
| **Resume mode for `/design-doc-create`** (after `/design-doc-interview`) | A resume-mode Drafter spawn writes only `drafter-resume-1.md`. It does NOT touch `drafter-1.md` (the slug differs: `drafter` vs `drafter-resume`). If `drafter-resume-1.md` already exists from an earlier resume attempt in the same folder, it is overwritten in place per the standard overwrite rule; `drafter-1.md` is left undisturbed. |

### `base-dir` Skill Migration

| Aspect | Decision |
|:--|:--|
| **Source path** | `~/.claude/skills/base-dir/SKILL.md` (19 lines today). |
| **Destination path** | `cafleet/.claude/skills/base-dir/SKILL.md`. |
| **Migration mode** | **Move only** — the repo copy is the sole copy after migration. |
| **Skills directory choice** | `cafleet/.claude/skills/` (the repo-local skills dir), not `cafleet/skills/` (the plugin-published dir). Rationale: `base-dir` is consumed by skills from *both* directories, but `Skill()` resolution can locate a repo-local skill from anywhere in the repo. Placing it in `.claude/skills/` matches the convention used by other repo-local helpers (`research-report`, `research-presentation`, `update-readme`). |
| **Content** | Identical to the current global file. No procedural changes. The new file at `cafleet/.claude/skills/base-dir/SKILL.md` has the same 19-line content. |
| **Global-copy removal** | The user manually deletes `~/.claude/skills/base-dir/` **after** the migration commit lands and `/research-report` smoke-test passes. The design doc cannot delete files outside the cafleet repo; this is documented as a follow-up operator step (see Implementation Step 5). |

`Skill(base-dir)` invocations in the five consuming skills do **not** change syntax — they remain bare `Skill(base-dir)` (not `Skill(cafleet:base-dir)`). The Claude Code skill loader resolves the bare name against the repo-local `.claude/skills/` first, then falls back to other plugin directories.

### Failure-Mode Reasoning

| Scenario | Behavior |
|:--|:--|
| `Skill(base-dir)` fails to load post-migration (e.g., the user has neither the global nor the repo copy installed). | Step 0 of every consuming skill aborts immediately — same failure mode as today, with the global copy missing. The audit feature is unreachable because `BASE` was never resolved. |
| `<base-dir>/spawn-prompts/` write fails (e.g., disk full, permission denied). | The Director surfaces the `Write` error to the user via `AskUserQuestion` (genuine technical failure). The spawn that just succeeded is NOT rolled back — the member is already live in its pane. The Director chooses between (a) continuing without the audit file (degraded mode) and (b) aborting the whole skill invocation per user direction. This branch is rare and explicitly out of the design-doc-execute happy path. |
| A `prompts/<role>.md` file is missing at render time. | Hard error — the Director aborts the spawn for that role with a clear message ("`prompts/<role>.md` not found in skills/<skill>/"). This is a regression bug in the skill, not a runtime fallback. |

### Coordination Protocol Compatibility

The design doc does NOT introduce new verbs, new pointer forms, or new `COMMENT(role)` markers. The five consuming skills' existing coordination protocols (verb + pointer schema in `/design-doc-create` and `/design-doc-execute`; free-form clarification-phase exemptions) are untouched. Audit-file writes are a Director-side side effect with no on-broker representation — they do not appear in the timeline, and the design doc itself never references them in a `cafleet message send` body.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation Audit & Top-Level References

`base-dir` is an internal skill — it is loaded by consuming skills via `Skill(base-dir)` and is explicitly NOT user-invocable. It therefore does NOT belong in the "Project Skills" lists in `cafleet/CLAUDE.md` or `cafleet/.claude/CLAUDE.md`, which enumerate user-invocable slash commands only. Claude Code's system-reminder surfaces `base-dir` to teammates that need it without any project-manifest entry.

- [ ] Grep `cafleet/README.md` (and `cafleet/ARCHITECTURE.md` and `cafleet/docs/` if present) for the strings `base-dir` and `~/.claude/skills/base-dir`. Update any reference to point at the new repo-local path `cafleet/.claude/skills/base-dir/`. If nothing is found, the task is no-op and gets checked off as such. <!-- completed: -->
- [ ] Verify `cafleet/CLAUDE.md` and `cafleet/.claude/CLAUDE.md` do NOT add a `/base-dir` entry to their Project Skills sections. (The skill is non-user-invocable; the system-reminder is its sole discovery path.) <!-- completed: -->

### Step 2: Migrate `base-dir` Skill

- [ ] Create `cafleet/.claude/skills/base-dir/SKILL.md` with the exact 19-line content of `~/.claude/skills/base-dir/SKILL.md`. <!-- completed: -->
- [ ] Verify the new file is picked up by Claude Code skill discovery: open a fresh Claude Code session in the cafleet repo and confirm `base-dir` appears in the system-reminder skill list. (Manual smoke step before Step 3 proceeds.) <!-- completed: -->

### Step 3: Pre-Flatten Brace Audit

- [ ] For each of the five consuming skills, grep `SKILL.md` and every `roles/*.md` file under that skill for literal `{` and `}` characters. Every occurrence outside the four cafleet kwargs (`{session_id}`, `{agent_id}`, `{director_name}`, `{director_agent_id}`) MUST already be doubled (`{{` / `}}`). Fix any unescaped occurrence in-place before Step 4 proceeds. This is a one-shot audit; once Step 4 flattens the role docs, the audit is permanently consumed. <!-- completed: -->

### Step 4: Introduce `prompts/<role>.md` Files (Flatten roles/)

For each of the five consuming skills, perform the same three-task sub-sequence. Order within Step 4 does not matter; skills are independent.

#### Step 4a: research-report

- [ ] Create `cafleet/.claude/skills/research-report/prompts/manager.md` by concatenating: (i) the Manager spawn-prompt block currently inline in `SKILL.md` § *2c. Spawn the Manager*, with the `[ROLE DEFINITION]` placeholder replaced by the verbatim content of `roles/manager.md`. Then delete `roles/manager.md`. <!-- completed: -->
- [ ] Repeat for `prompts/scout.md` (source: SKILL.md § *Step 3* Scout spawn prompt + `roles/scout.md`) and delete `roles/scout.md`. <!-- completed: -->
- [ ] Repeat for `prompts/researcher.md` (source: SKILL.md § *4b. Spawn each Researcher* + `roles/researcher.md`) and delete `roles/researcher.md`. <!-- completed: -->

#### Step 4b: research-presentation

- [ ] Create `cafleet/.claude/skills/research-presentation/prompts/presentation.md` from the Presentation spawn-prompt block + `roles/presentation.md`. Delete `roles/presentation.md`. <!-- completed: -->
- [ ] Create `prompts/transcript.md` from the Transcript spawn-prompt block + `roles/transcript.md`. Delete `roles/transcript.md`. <!-- completed: -->
- [ ] Create `prompts/visual-reviewer.md` from the VR spawn-prompt block + `roles/visual-reviewer.md`. Delete `roles/visual-reviewer.md`. <!-- completed: -->

#### Step 4c: design-doc-create

- [ ] Create `cafleet/skills/design-doc-create/prompts/drafter.md` from the **normal-mode** Drafter spawn-prompt block + `roles/drafter.md`. <!-- completed: -->
- [ ] Create `prompts/drafter-resume.md` from the **resume-mode** Drafter spawn-prompt block + `roles/drafter.md` (same role definition, different prologue / instructions). Then delete `roles/drafter.md`. <!-- completed: -->
- [ ] Create `prompts/reviewer.md` from the Reviewer spawn-prompt block + `roles/reviewer.md`. Delete `roles/reviewer.md`. <!-- completed: -->

#### Step 4d: design-doc-execute

- [ ] Create `cafleet/skills/design-doc-execute/prompts/programmer.md` from the Programmer spawn-prompt block + `roles/programmer.md`. Delete `roles/programmer.md`. <!-- completed: -->
- [ ] Create `prompts/tester.md` from the Tester spawn-prompt block + `roles/tester.md`. Delete `roles/tester.md`. <!-- completed: -->
- [ ] Create `prompts/verifier.md` from the Verifier spawn-prompt block + `roles/verifier.md`. Delete `roles/verifier.md`. <!-- completed: -->

#### Step 4e: design-doc-interview

- [ ] Create `cafleet/skills/design-doc-interview/prompts/analyzer.md` from the Analyzer spawn-prompt block + `roles/analyzer.md`. Delete `roles/analyzer.md`. <!-- completed: -->

### Step 5: Update SKILL.md Director Instructions

For each of the five consuming skills, replace the **inline spawn-prompt fenced blocks** in `SKILL.md` with a short instruction pointing the Director at the `prompts/<role>.md` file. The audit-file write step is appended.

Reference template for the replacement instruction (adapt per skill — outer fence is four backticks so the inner ```bash fence renders correctly):

````
#### <step>. Spawn the <Role>

1. Read `prompts/<role>.md`.
2. Substitute every `[INSERT ...]` marker with the corresponding skill-side value:
   - `[INSERT today's date]` → today's date in `YYYY-MM-DD`.
   - `[INSERT ${OUTPUT_DIR}]` → the absolute path resolved in Step 0.
   - … (skill-specific list)
3. Run:
   ```bash
   cafleet --session-id <session-id> --json member create --agent-id <director-agent-id> \
     --name "<role-or-role-NN>" \
     --description "..." \
     -- "<rendered prompt>"
   ```
4. Parse `agent_id` from the JSON response and substitute it for `<role-agent-id>` in all subsequent cafleet calls targeting this member.
5. **Write the audit file**: re-render the prompt locally with `prompt.format(session_id=…, agent_id=<new-member-id>, director_name=…, director_agent_id=…)`, and Write the result to `<base-dir>/spawn-prompts/<role>-<n>.md` (per-role slot ordinal, see § *Audit File Layout*).
````

- [ ] Update `research-report/SKILL.md`: replace the three inline spawn-prompt blocks (Manager, Scout, Researcher) with the read-render-spawn-audit pattern. <!-- completed: -->
- [ ] Update `research-presentation/SKILL.md`: replace the three inline blocks (Presentation, Transcript, Visual Reviewer). <!-- completed: -->
- [ ] Update `design-doc-create/SKILL.md`: replace the three inline blocks (Drafter normal, Drafter resume, Reviewer). The two Drafter modes share the same Step 1d location; the resume-mode branch reads `prompts/drafter-resume.md` instead of `prompts/drafter.md`. <!-- completed: -->
- [ ] Update `design-doc-execute/SKILL.md`: replace the three inline blocks (Programmer, Tester, Verifier). <!-- completed: -->
- [ ] Update `design-doc-interview/SKILL.md`: replace the single Analyzer inline block. <!-- completed: -->

### Step 6: Smoke-Test the Migration

- [ ] Run `/research-report` with any short topic phrase (e.g., `"tcp-vs-udp"`) end-to-end in a fresh tmux session. Confirm: (i) `Skill(base-dir)` resolves correctly, (ii) every spawned member starts successfully, (iii) `<base-dir>/spawn-prompts/` contains `manager-1.md`, plus per-role files for every Scout and Researcher actually spawned, (iv) audit-file contents match the prompts the panes received. <!-- completed: -->

**Operator follow-up (outside this design doc's authority — no checkbox).** After Step 6 passes, the operator manually removes `~/.claude/skills/base-dir/` so the repo copy is the sole copy on disk. This action lives outside the cafleet repo and cannot be performed by the implementation team; Success Criterion #3 is satisfied by the operator at their convenience, independent of when the design doc is marked Complete.

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-11 | Initial draft |
| 2026-05-11 | Round-1 review: clarified slot-based counter ownership and source of truth; documented stale-file accept-as-staleness policy; specified `str.format()` for audit re-render; resolved resume-mode overwrite contradiction; promoted brace audit to Step 3; relocated base-dir manifest tasks (skill is non-user-invocable); fixed nested-fence rendering bug; moved global-skill removal into operator-follow-up paragraph; corrected smoke-test wording and task count to 0/24. |
