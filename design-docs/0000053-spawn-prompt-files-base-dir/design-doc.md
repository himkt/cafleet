# Spawn-Prompt Files and base-dir Migration

**Status**: Approved
**Progress**: 0/27 tasks complete
**Last Updated**: 2026-05-11

## Overview

Externalize CAFleet member spawn prompts from inline `SKILL.md` blocks into per-role `prompts/<role>.md` files, and relocate the `base-dir` skill from the user's global `~/.claude/skills/base-dir/` into the repo-root `skills/base-dir/` so the cafleet plugin becomes self-contained (the migrated copy ships as `cafleet:base-dir` via both `.claude-plugin/plugin.json` and `.codex-plugin/plugin.json`).

## Success Criteria

- [ ] Every base-dir-consuming skill (`research-report`, `research-presentation`, `design-doc-create`, `design-doc-execute`, `design-doc-interview`) reads its member spawn prompts from `prompts/<role>.md` files instead of `SKILL.md`-inline blocks.
- [ ] The `base-dir` skill exists at `skills/base-dir/SKILL.md` and ships as `cafleet:base-dir` via both plugin manifests.
- [ ] Existing `roles/<role>.md` files in the five consuming skills are flattened into the corresponding `prompts/<role>.md` files and deleted; `SKILL.md` references to `roles/...` are updated to point at `prompts/...`.
- [ ] A `/research-report` smoke run end-to-end succeeds with the new layout.

---

## Background

Today, CAFleet member spawn prompts live in two places per skill:

1. **`SKILL.md`** carries the spawn-prompt template inline (e.g., a fenced block titled "Manager spawn prompt:" containing `[ROLE DEFINITION]` placeholders, `[INSERT ...]` markers, and the four cafleet kwargs `{session_id}` / `{agent_id}` / `{director_name}` / `{director_agent_id}`).
2. **`roles/<role>.md`** carries the role definition that the Director injects verbatim into the `[ROLE DEFINITION]` block at assembly time.

Two pain points:

- **Plugin self-containment gap.** `base-dir` lives only at `~/.claude/skills/base-dir/`, so operators who install the cafleet plugin without the global copy see `Skill(base-dir)` fail to resolve, and every consuming skill's Step 0 breaks. The cafleet plugin should ship its own copy.
- **Spawn-prompt locality.** Spawn-prompt templates live inline inside every consuming `SKILL.md`, which makes them invisible to maintainer audit (they cannot be `Read` as standalone files, diffed in PRs as separate documents, or re-rendered for inspection) and forces every `SKILL.md` edit to also re-validate the prompt block. Moving the templates to per-role `prompts/<role>.md` files separates orchestration documentation from spawn-prompt content.

The two changes are bundled in one design doc because they touch the same files (every consuming-skill `SKILL.md`) and share a single migration commit boundary.

---

## Specification

### Coverage

In scope: the five base-dir-consuming skills that spawn members.

| Skill | Skills dir | Spawned members | Existing `roles/<role>.md` files (to flatten) |
|:--|:--|:--|:--|
| `research-report` | `skills/research-report/` | Manager, Scout, Researcher | `roles/manager.md`, `roles/scout.md`, `roles/researcher.md` |
| `research-presentation` | `skills/research-presentation/` | Presentation, Transcript, Visual Reviewer | `roles/presentation.md`, `roles/transcript.md`, `roles/visual-reviewer.md` |
| `design-doc-create` | `skills/design-doc-create/` | Drafter (two modes: normal, resume), Reviewer | `roles/drafter.md`, `roles/reviewer.md` |
| `design-doc-execute` | `skills/design-doc-execute/` | Programmer, Tester, Verifier | `roles/programmer.md`, `roles/tester.md`, `roles/verifier.md` |
| `design-doc-interview` | `skills/design-doc-interview/` | Analyzer | `roles/analyzer.md` |

Out of scope (do NOT introduce a `prompts/` folder): `agent-team-monitoring`, `agent-team-supervision`, `my-slidev`, `create-figure`, `cafleet`. These skills are not base-dir consumers.

Non-spawned `roles/director.md` files document the main-Claude Director's behavior. They are NOT spawn-prompt templates and are NOT migrated into `prompts/`. They remain in `roles/` as reference documentation and are linked from `SKILL.md` unchanged. Per-skill enumeration of post-migration `roles/` survivors:

- `research-report/roles/director.md` — exists, survives.
- `research-presentation/roles/director.md` — exists, survives.
- `design-doc-create/roles/director.md` — exists, survives.
- `design-doc-execute/roles/director.md` — exists, survives.
- `design-doc-interview/roles/` — contains no `director.md`. After Step 4e flattens `analyzer.md`, the `roles/` directory has no surviving content and is deleted by an explicit Step 4e clean-up sub-task (otherwise the empty directory would be mistakable for an in-progress state).

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
2. A `<ROLE DEFINITION>...</ROLE DEFINITION>` block with the role-definition content **inlined verbatim** (flattened from the old `roles/<role>.md` file). Preserve the `<ROLE DEFINITION>` / `</ROLE DEFINITION>` tags around the inlined content as a visual delimiter; no functional substitution occurs on the tags at spawn time.
3. The "Load these skills at startup:" list, unchanged.
4. The literal-UUID identity block (`SESSION ID: {session_id}`, `DIRECTOR AGENT ID: {director_agent_id}`, `YOUR AGENT ID: {agent_id}`, etc.).
5. Skill-side `[INSERT ...]` markers for values the Director substitutes per spawn (`[INSERT today's date]`, `[INSERT output directory]`, `[INSERT USER'S ORIGINAL REQUEST]`, `[INSERT DESIGN DOCUMENT PATH]`, `[INSERT YOUR ASSIGNMENT]`, `[INSERT YOUR TASK ID]`, etc.).
6. The communication protocol block, unchanged.
7. Any trailing instructions, unchanged.

### Three-Tier Variable Substitution Model

The existing three-tier model is preserved verbatim. Every `prompts/<role>.md` file mixes three kinds of placeholders, resolved in this order:

| Tier | Marker syntax | Substituted by | When | Example |
|:--|:--|:--|:--|:--|
| 1. Skill-side fill-in | `[INSERT <description>]` | The Director, **before** the `member create` call | Per-spawn (skill-specific values) | `[INSERT today's date]`, `[INSERT output directory]`, `[INSERT USER'S ORIGINAL REQUEST]` |
| 2. Verbatim role inlining | `<ROLE DEFINITION>...content...</ROLE DEFINITION>` | (none — content is **already inlined** post-flatten; the tag pair stays as a visual delimiter) | Authoring time (one-shot at migration) | n/a after flatten |
| 3. cafleet kwargs | `{session_id}`, `{agent_id}`, `{director_name}`, `{director_agent_id}` | `cafleet member create` via Python `str.format()` on the entire prompt string | At spawn time | Single-braced; never doubled |

The tier-1 marker syntax is a **notational convention** — `[INSERT <description>]` means "substitute the value the consuming skill computed earlier in its procedure." The Director performs the substitution in-memory before passing the rendered prompt to `cafleet member create`; no shell variable expansion is involved.

Authoring rule for `prompts/<role>.md` files: tier-3 single-braced kwargs are the **only** literal `{` / `}` characters permitted in the file. Any other `{` or `}` (e.g., a JSON example inside the role definition) MUST be doubled to `{{` / `}}` before being written into the file — `str.format()` raises `KeyError` at spawn time otherwise. This is the same rule the existing inline `SKILL.md` blocks already enforce in principle, but it has not been audited across the role docs that are about to be flattened. Implementation **Step 3** runs a one-shot brace audit over every inline `SKILL.md` spawn-prompt block and every `roles/<role>.md` file, doubles any stray unescaped `{` / `}`, and only then permits the Step 4 flatten — so the migration does not surface a latent `KeyError` at the first spawn after merge.

### Director Render Procedure

After the migration, the Director's spawn flow becomes:

1. **Read** `prompts/<role>.md` from disk (`Read` tool) — the path is relative to the skill directory.
2. **Substitute tier-1 `[INSERT ...]` markers** with skill-side values (string replace, in memory). The Director MUST resolve every `[INSERT ...]` marker; any unresolved marker is a bug, not a fallthrough.
3. **Pass the result** as the positional spawn-prompt argument to `cafleet --json member create --agent-id <director-agent-id> -- "<rendered prompt>"`. cafleet's own `_resolve_prompt` performs tier-3 `str.format()` on this string with the four kwargs.
4. **Parse `agent_id`** from the `member create` JSON response. Treat it as the new member's literal UUID for all subsequent `cafleet` calls.

### `base-dir` Skill Migration

| Aspect | Decision |
|:--|:--|
| **Source path** | `~/.claude/skills/base-dir/SKILL.md` (19 lines today). |
| **Destination path** | `skills/base-dir/SKILL.md` (repo-root plugin source; ships as `cafleet:base-dir` via both `.claude-plugin/plugin.json` and `.codex-plugin/plugin.json`). |
| **Migration mode** | **Move only** — the plugin-shipped copy is the sole copy after migration. |
| **Skills directory choice** | `skills/` (the repo-root plugin source). Rationale: ship base-dir as part of the cafleet plugin under `skills/base-dir/` so it auto-installs alongside the consuming skills (`cafleet:research-report`, `cafleet:my-slidev`, `cafleet:design-doc-create`, `cafleet:design-doc-execute`, `cafleet:design-doc-interview`). Consuming skills invoke `Skill(cafleet:base-dir)` (prefixed) post-migration. Choosing `skills/` over `.claude/skills/` follows the precedent set by design 0000054 for previously-project-local skills that grew plugin-shipping requirements; `.claude/skills/` is now reserved for skills that remain project-local (currently only `update-readme/`). |
| **Content** | Identical to the current global file. No procedural changes. The new file at `skills/base-dir/SKILL.md` has the same 19-line content. |
| **Global-copy removal** | The user manually deletes `~/.claude/skills/base-dir/` **after** the migration commit lands and `/research-report` smoke-test passes. The design doc cannot delete files outside the cafleet repo; this is documented as a follow-up operator step (see Implementation Step 6). |

Consuming skills MUST invoke `Skill(cafleet:base-dir)` (prefixed) post-migration. The plugin loader registers the skill under the `cafleet:` namespace; the bare-name `Skill(base-dir)` form will not resolve once the global copy at `~/.claude/skills/base-dir/` is removed. Every base-dir consumer (`research-report`, `research-presentation`, `design-doc-create`, `design-doc-execute`, `design-doc-interview`) must be updated as part of Step 5 to use the prefixed form.

### Failure-Mode Reasoning

| Scenario | Behavior |
|:--|:--|
| `Skill(cafleet:base-dir)` fails to load post-migration (e.g., the user has neither the global nor the repo copy installed). | Step 0 of every consuming skill aborts immediately — same failure mode as today, with the global copy missing. |
| A `prompts/<role>.md` file is missing at render time. | Hard error — the Director aborts the spawn for that role with a clear message ("`prompts/<role>.md` not found in skills/<skill>/"). This is a regression bug in the skill, not a runtime fallback. |

### Coordination Protocol Compatibility

The design doc does NOT introduce new verbs, new pointer forms, or new `COMMENT(role)` markers. The five consuming skills' existing coordination protocols (verb + pointer schema in `/design-doc-create` and `/design-doc-execute`; free-form clarification-phase exemptions) are untouched.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation Audit & Top-Level References

`base-dir` is an internal skill — it is loaded by consuming skills via `Skill(cafleet:base-dir)` (prefixed) and is explicitly NOT user-invocable. It therefore does NOT belong in the "Project Skills" lists in `CLAUDE.md` or `.claude/CLAUDE.md`, which enumerate user-invocable slash commands only. Claude Code's system-reminder surfaces `cafleet:base-dir` to teammates that need it without any project-manifest entry.

- [ ] Grep `README.md` (and `ARCHITECTURE.md` and `docs/` if present) for the strings `base-dir`, `~/.claude/skills/base-dir`, and `Skill(base-dir)`. Update any path reference to point at the new repo-root plugin source `skills/base-dir/`. Update any `Skill(base-dir)` invocation to the prefixed `Skill(cafleet:base-dir)` form. If nothing is found, the task is no-op and gets checked off as such. <!-- completed: -->
- [ ] Verify `CLAUDE.md` and `.claude/CLAUDE.md` do NOT add a `/base-dir` entry to their Project Skills sections. (The skill is non-user-invocable; the system-reminder is its sole discovery path.) <!-- completed: -->

### Step 2: Migrate `base-dir` Skill

- [ ] Create `skills/base-dir/SKILL.md` with the exact 19-line content of `~/.claude/skills/base-dir/SKILL.md`. <!-- completed: -->
- [ ] Edit `.claude-plugin/plugin.json`: append `"./skills/base-dir"` to the `skills` array. Bump `version` per the semver policy in design 0000054 (adding a shipped skill is a MINOR bump). Edit `.codex-plugin/plugin.json`: bump `version` to match; the `"skills": "./skills/"` auto-discovery field is unchanged. Edit `cafleet/pyproject.toml`: bump the version to match the plugin manifests. <!-- completed: -->
- [ ] Run `mise //:sync-skills` so the home-directory working copy at `~/.claude/skills/base-dir/` is synchronized from the new plugin-source copy. (Design 0000054 acknowledges the home-directory working copy is operator-managed; this sync keeps the maintainer's local environment consistent.) <!-- completed: -->
- [ ] Verify the new file is picked up by Claude Code skill discovery: open a fresh Claude Code session in the cafleet repo and confirm `cafleet:base-dir` (prefixed) appears in the system-reminder skill list. (Manual smoke step before Step 3 proceeds.) <!-- completed: -->

### Step 3: Pre-Flatten Brace Audit

- [ ] For each of the five consuming skills, grep `SKILL.md` and every `roles/*.md` file under that skill for literal `{` and `}` characters. Every occurrence outside the four cafleet kwargs (`{session_id}`, `{agent_id}`, `{director_name}`, `{director_agent_id}`) MUST already be doubled (`{{` / `}}`). Fix any unescaped occurrence in-place before Step 4 proceeds. This is a one-shot audit; once Step 4 flattens the role docs, the audit is permanently consumed. <!-- completed: -->

### Step 4: Introduce `prompts/<role>.md` Files (Flatten roles/)

For each of the five consuming skills, perform the same three-task sub-sequence. Order across Step 4 skills (4a-4e) does not matter; skills are independent. Order WITHIN a single skill's sub-steps may matter — see Step 4c for an explicit example. After all `skills/<skill>/prompts/...md` files are created and all `roles/<role>.md` files deleted, run `mise //:sync-skills` so the maintainer's `~/.claude/skills/` working copies stay consistent with the plugin source.

#### Step 4a: research-report

- [ ] Create `skills/research-report/prompts/manager.md` by concatenating: (i) the Manager spawn-prompt block currently inline in `SKILL.md` § *2c. Spawn the Manager*, with the `[ROLE DEFINITION]` placeholder replaced by the verbatim content of `skills/research-report/roles/manager.md`. Then delete `skills/research-report/roles/manager.md`. <!-- completed: -->
- [ ] Repeat for `skills/research-report/prompts/scout.md` (source: SKILL.md § *Step 3* Scout spawn prompt + `skills/research-report/roles/scout.md`) and delete `skills/research-report/roles/scout.md`. <!-- completed: -->
- [ ] Repeat for `skills/research-report/prompts/researcher.md` (source: SKILL.md § *4b. Spawn each Researcher* + `skills/research-report/roles/researcher.md`) and delete `skills/research-report/roles/researcher.md`. <!-- completed: -->

#### Step 4b: research-presentation

- [ ] Create `skills/research-presentation/prompts/presentation.md` from the Presentation spawn-prompt block + `skills/research-presentation/roles/presentation.md`. Delete `skills/research-presentation/roles/presentation.md`. <!-- completed: -->
- [ ] Create `skills/research-presentation/prompts/transcript.md` from the Transcript spawn-prompt block + `skills/research-presentation/roles/transcript.md`. Delete `skills/research-presentation/roles/transcript.md`. <!-- completed: -->
- [ ] Create `skills/research-presentation/prompts/visual-reviewer.md` from the VR spawn-prompt block + `skills/research-presentation/roles/visual-reviewer.md`. Delete `skills/research-presentation/roles/visual-reviewer.md`. <!-- completed: -->

#### Step 4c: design-doc-create

**Sub-step precedence**: sub-step 1 (create `prompts/drafter.md`) MUST precede sub-step 2 (create `prompts/drafter-resume.md`), because sub-step 2 deletes `roles/drafter.md` which sub-step 1 still needs to read. Sub-steps 1 and 2 must both precede sub-step 3 only if sub-step 3 also reads `roles/drafter.md` — it does not (sub-step 3 uses `roles/reviewer.md`), so sub-step 3 is order-independent of sub-steps 1 and 2.

- [ ] Create `skills/design-doc-create/prompts/drafter.md` from the **normal-mode** Drafter spawn-prompt block + `skills/design-doc-create/roles/drafter.md`. <!-- completed: -->
- [ ] Create `skills/design-doc-create/prompts/drafter-resume.md` from the **resume-mode** Drafter spawn-prompt block + `skills/design-doc-create/roles/drafter.md` (same role definition, different prologue / instructions). Then delete `skills/design-doc-create/roles/drafter.md`. <!-- completed: -->
- [ ] Create `skills/design-doc-create/prompts/reviewer.md` from the Reviewer spawn-prompt block + `skills/design-doc-create/roles/reviewer.md`. Delete `skills/design-doc-create/roles/reviewer.md`. <!-- completed: -->

#### Step 4d: design-doc-execute

- [ ] Create `skills/design-doc-execute/prompts/programmer.md` from the Programmer spawn-prompt block + `skills/design-doc-execute/roles/programmer.md`. Delete `skills/design-doc-execute/roles/programmer.md`. <!-- completed: -->
- [ ] Create `skills/design-doc-execute/prompts/tester.md` from the Tester spawn-prompt block + `skills/design-doc-execute/roles/tester.md`. Delete `skills/design-doc-execute/roles/tester.md`. <!-- completed: -->
- [ ] Create `skills/design-doc-execute/prompts/verifier.md` from the Verifier spawn-prompt block + `skills/design-doc-execute/roles/verifier.md`. Delete `skills/design-doc-execute/roles/verifier.md`. <!-- completed: -->

#### Step 4e: design-doc-interview

- [ ] Create `skills/design-doc-interview/prompts/analyzer.md` from the Analyzer spawn-prompt block + `skills/design-doc-interview/roles/analyzer.md`. Delete `skills/design-doc-interview/roles/analyzer.md`. <!-- completed: -->
- [ ] After the analyzer flatten and `roles/analyzer.md` delete, also delete the now-empty `skills/design-doc-interview/roles/` directory (no non-spawned roles survive in this skill). <!-- completed: -->

### Step 5: Update SKILL.md Director Instructions

For each of the five consuming skills, replace the **inline spawn-prompt fenced blocks** in `SKILL.md` with a short instruction pointing the Director at the `prompts/<role>.md` file. Each task also updates `Skill(base-dir)` invocations in the affected SKILL.md to the prefixed `Skill(cafleet:base-dir)` form — bare-name resolution does not work post-migration once the global `~/.claude/skills/base-dir/` copy is removed by the operator.

Reference template for the replacement instruction (adapt per skill — outer fence is four backticks so the inner ```bash fence renders correctly):

````
#### <step>. Spawn the <Role>

1. Read `prompts/<role>.md`.
2. Substitute every `[INSERT ...]` marker with the corresponding skill-side value:
   - `[INSERT today's date]` → today's date in `YYYY-MM-DD`.
   - `[INSERT output directory]` → the absolute path resolved in Step 0.
   - … (skill-specific list)
3. Run:
   ```bash
   cafleet --session-id <session-id> --json member create --agent-id <director-agent-id> \
     --name "<role-or-role-NN>" \
     --description "..." \
     -- "<rendered prompt>"
   ```
4. Parse `agent_id` from the JSON response and substitute it for `<role-agent-id>` in all subsequent cafleet calls targeting this member.
````

- [ ] Update `research-report/SKILL.md`: replace the three inline spawn-prompt blocks (Manager, Scout, Researcher) with the read-render-spawn pattern. Update `Skill(base-dir)` invocations to `Skill(cafleet:base-dir)`. <!-- completed: -->
- [ ] Update `research-presentation/SKILL.md`: replace the three inline blocks (Presentation, Transcript, Visual Reviewer). Update `Skill(base-dir)` invocations to `Skill(cafleet:base-dir)`. <!-- completed: -->
- [ ] Update `design-doc-create/SKILL.md`: replace the three inline blocks (Drafter normal, Drafter resume, Reviewer). The two Drafter modes share the same Step 1d location; the resume-mode branch reads `prompts/drafter-resume.md` instead of `prompts/drafter.md`. Update `Skill(base-dir)` invocations to `Skill(cafleet:base-dir)`. <!-- completed: -->
- [ ] Update `design-doc-execute/SKILL.md`: replace the three inline blocks (Programmer, Tester, Verifier). Update `Skill(base-dir)` invocations to `Skill(cafleet:base-dir)`. <!-- completed: -->
- [ ] Update `design-doc-interview/SKILL.md`: replace the single Analyzer inline block. Update `Skill(base-dir)` invocations to `Skill(cafleet:base-dir)`. <!-- completed: -->

### Step 6: Smoke-Test the Migration

- [ ] Run `/research-report` with a short topic phrase (e.g., `"tcp-vs-udp"`) end-to-end in a fresh tmux session inside a fresh Claude Code instance that has the cafleet plugin installed. Confirm: (i) `Skill(cafleet:base-dir)` resolves correctly from the plugin location, (ii) every spawned member starts successfully, (iii) the spawn prompts read from `skills/research-report/prompts/<role>.md` files are passed to each member (verify by `cafleet member capture` on a Researcher pane and confirming the prompt text matches the file content). <!-- completed: -->

**Operator follow-up (outside this design doc's authority — no checkbox).** After Step 6 passes, the operator manually removes `~/.claude/skills/base-dir/` so the repo-root plugin source `skills/base-dir/` (installed as `cafleet:base-dir`) is the sole copy on disk. This action lives outside the cafleet repo and cannot be performed by the implementation team; it is documented here as an operator follow-up and is intentionally not gated by a Success Criterion checkbox (the in-repo deliverable in SC #2 — skill exists at `skills/base-dir/SKILL.md` and ships as `cafleet:base-dir` via both plugin manifests — is the verifiable boundary).

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-11 | Initial draft |
| 2026-05-11 | Round-1 review: promoted brace audit to Step 3; relocated base-dir manifest tasks (skill is non-user-invocable); fixed nested-fence rendering bug; moved global-skill removal into operator-follow-up paragraph. |
| 2026-05-11 | Resume-mode revision: applied interview Q1-Q12 answers; dropped audit-file feature scope; rewrote base-dir migration target to skills/base-dir/ plugin destination; updated Coverage table and Step 4 paths to post-0054 layout. |
| 2026-05-11 | Round-2 review: reverted Status to Draft pending Reviewer approval; trimmed SC #2 to the in-repo deliverable only (global-copy removal lives solely in the Step 6 operator-follow-up paragraph); enumerated post-migration roles/director.md survivors per skill and added Step 4e clean-up sub-task to delete the empty design-doc-interview/roles/ directory; clarified that the `<ROLE DEFINITION>` tag pair is preserved around the inlined content; dropped sub-clause (iv) from the Step 6 smoke test (no need to verify absence of a never-implemented feature); trimmed the Round-1 changelog row to surviving items; task count 0/26 → 0/27 for the new Step 4e clean-up task. |
| 2026-05-11 | User approval — Status promoted from Draft to Approved; ready for `/design-doc-execute`. |
| 2026-05-11 | Doc reconstructed from conversation transcript after an in-session `git checkout` clobbered uncommitted revision changes. Content matches the user-approved Round-2 state; the Round-1 and Resume-mode and Round-2 changelog rows above describe edits that physically passed through the live Drafter/Reviewer loop but exist only as transcript-reconstructed content on disk. |
