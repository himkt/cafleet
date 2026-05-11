# Promote Project-Local Skills and Agents to Global

**Status**: Complete
**Progress**: 49/49 tasks complete
**Last Updated**: 2026-05-11

## Overview

Move four project-local skills (`create-figure`, `my-slidev`, `research-presentation`, `research-report`) from `.claude/skills/` to `~/.claude/skills/` so they are reusable across projects, embed the two project-local agents (`slide-creator`, `web-researcher`) inline inside their consuming skills so the cafleet plugin can ship them through the existing skill packaging path, and add the four moved skills to both plugin manifests. The `/update-readme` skill stays project-local.

## Success Criteria

- [ ] `~/.claude/skills/` contains working copies of `create-figure/`, `my-slidev/`, `research-presentation/`, `research-report/`.
- [ ] `.claude/skills/` (project-local) contains only `update-readme/`.
- [ ] `.claude/agents/` is removed entirely (both `slide-creator.md` and `web-researcher.md` deleted).
- [ ] `slide-creator` spec is embedded in `my-slidev/SKILL.md`; `web-researcher` spec is embedded in `research-report/SKILL.md`.
- [ ] Both `.claude-plugin/plugin.json` and `.codex-plugin/plugin.json` bundle the four moved skills.
- [ ] Plugin source tree `skills/` (at the repo root) contains synchronized copies of the four moved skills, so the marketplace tarball actually ships them under the `cafleet:*` namespace.
- [ ] `CLAUDE.md`, `.claude/CLAUDE.md`, `.claude/settings.json`, and every `.claude/rules/*.md` file contain no reference to the moved skills or agents — only the `/update-readme` bullet remains.
- [ ] All four moved skills load and execute correctly when invoked via `Skill(<name>)` from a directory outside the cafleet repo (no project-rooted path or `mise //...` dependency).
- [ ] `my-slidev/theme/` assets are self-contained inside the skill directory; no reference resolves to the cafleet repo root.
- [ ] `/create-figure` self-bootstraps its matplotlib dependency via `uv run --with matplotlib python <script>` without requiring mise or a project-rooted `research` uv dependency group.
- [ ] From a fresh codex CLI session, the four new skills are discovered via `.codex-plugin/plugin.json` auto-discovery, and the embedded `slide-creator` / `web-researcher` specs are invocable via the documented dispatch recipe.

---

## Background

CAFleet currently distributes seven `cafleet:*` skills through the marketplace plugin (`.claude-plugin/plugin.json` + `.codex-plugin/plugin.json`). A second tier of skills (`create-figure`, `my-slidev`, `research-presentation`, `research-report`, `update-readme`) and two agents (`slide-creator`, `web-researcher`) currently live inside the repo at `.claude/skills/` and `.claude/agents/`, making them unavailable when the maintainer works in other projects.

Two backend constraints shape the distribution strategy:

| Constraint | Provenance | Impact |
|:--|:--|:--|
| Codex CLI plugins do not support standalone subagents. | User-supplied in the design-doc-create clarification round (see *Decisions* § D1 below). The Programmer SHOULD reconfirm against the current `codex` upstream docs at implementation time before the embed-strategy lands; if the constraint has been lifted, escalate via `COMMENT(programmer)` so the embedding can be revisited. | Agents shipped as standalone `.md` files are invisible to codex users who install the cafleet plugin. |
| Codex CLI does not load `~/.claude/agents/` on its own. | User-supplied in the same clarification round (see *Decisions* § D1). Same reconfirmation guidance applies. | The only viable agent distribution channel that reaches codex is **embedding the agent spec inside a skill** the codex plugin already ships. |

`/update-readme` is intentionally excluded from the move because it is project-specific (it consumes `ARCHITECTURE.md` and `docs/` of *this* repo). This mirrors the precedent set by design-doc 0000048.

---

## Specification

### Decisions (user-supplied, design-doc-create clarification round, 2026-05-11)

Six clarifying questions and the user's verbatim answers — anchoring every Q-citation elsewhere in this document.

| # | Question | Answer |
|:--|:--|:--|
| D1 | Codex agent / plugin-subagent constraints. | Codex CLI plugins do not support standalone subagents. Codex CLI does not load `~/.claude/agents/` on its own. Only viable agent channel for codex is skill-embedded specs. |
| D2 | `/create-figure` invocation portability. | Promote globally; rewrite the SKILL.md invocation to `uv run --with matplotlib python <script>` so the skill self-bootstraps anywhere `uv` is on `$PATH`. |
| D3 | Plugin bundling scope after promotion. | Bundle all four moved skills (`create-figure`, `my-slidev`, `research-presentation`, `research-report`) via both `.claude-plugin/plugin.json` and `.codex-plugin/plugin.json`. Accept the duplicate-name risk on the maintainer machine (the maintainer sees both `<name>` from `~/.claude/skills/` and `cafleet:<name>` from the installed plugin). |
| D4 | Agent embedding location. | Embed each agent spec inline inside the SKILL.md of its consuming skill. Do NOT ship standalone agent `.md` files via the plugin. Drafter picks the host skill — `slide-creator` lives in `my-slidev`, `web-researcher` lives in `research-report` (justification in *Agent Embedding Strategy* below). |
| D5 | Documentation-cleanup policy. | Strict total removal per `~/.claude/rules/removal.md`. Delete every bullet for the moved skills/agents from `CLAUDE.md`, `.claude/CLAUDE.md`, `.claude/settings.json` `permissions.allow`, and rule cross-references. `/update-readme` bullet stays. No deprecation notices. |
| D6 | `/research-report` output path. | Unchanged — output to `<CWD>/researches/<topic-slug>/`. The SKILL.md MUST instruct callers to gitignore `researches/` per project. The skill does not modify `.gitignore` itself. |

### Inventory of Items to Migrate

| Kind | Name | Source Path (current) | Destination | Notes |
|:--|:--|:--|:--|:--|
| Skill | `create-figure` | `.claude/skills/create-figure/` | `~/.claude/skills/create-figure/` + `skills/create-figure/` (repo-root plugin source) | Rewrite invocation per D2; no mise dependency. |
| Skill | `my-slidev` | `.claude/skills/my-slidev/` | `~/.claude/skills/my-slidev/` + `skills/my-slidev/` | Rewrite cafleet-repo-rooted segment in `SKILL.md` line 8; embed `slide-creator` agent spec. |
| Skill | `research-presentation` | `.claude/skills/research-presentation/` | `~/.claude/skills/research-presentation/` + `skills/research-presentation/` | Add a discovery pointer to the embedded `slide-creator` spec in `my-slidev` (no literal `Agent(subagent_type=...)` call sites exist to rewrite — verified). |
| Skill | `research-report` | `.claude/skills/research-report/` | `~/.claude/skills/research-report/` + `skills/research-report/` | Embed `web-researcher` agent spec; document per-project `researches/` gitignore expectation per D6. |
| Skill | `update-readme` | `.claude/skills/update-readme/` | UNCHANGED (stays project-local) | Sole exception per user instruction. |
| Agent | `slide-creator` | `.claude/agents/slide-creator.md` | DELETED; spec embedded inline in `~/.claude/skills/my-slidev/SKILL.md` and `skills/my-slidev/SKILL.md` | Drop hard-coded "inside the cafleet repo" passage on lines 39 and 57 (verbatim text reproduced in Step 3 task 2 below). |
| Agent | `web-researcher` | `.claude/agents/web-researcher.md` | DELETED; spec embedded inline in `~/.claude/skills/research-report/SKILL.md` and `skills/research-report/SKILL.md` | No cafleet-repo path references in the source; clean embed. |

### Distribution Topology

Per D3, each moved skill ends up in **two** locations on disk:

```
~/.claude/skills/<name>/             # working copy on the maintainer machine; unprefixed Skill(<name>) invocation
skills/<name>/                       # plugin source at the repo root; shipped to others as cafleet:<name>
```

Both copies are byte-identical at any moment. The plugin source tree (`skills/<name>/`, at the repo root — *not* under the `cafleet/` Python package, *not* under `.claude/`) is the canonical commit target; `~/.claude/skills/<name>/` is treated as a working copy refreshed from the plugin source. A maintainer running `Skill(<name>)` inside the cafleet repo will see *both* registrations (`<name>` from `~/.claude/` and `cafleet:<name>` from the installed plugin). They are equivalent.

Both plugin manifests resolve skill paths **relative to the repo root**, not relative to the manifest file or any Python package:

- `.claude-plugin/plugin.json` lists `./skills/<name>` entries → repo-root `skills/<name>/`.
- `.codex-plugin/plugin.json` uses the glob `"skills": "./skills/"` → repo-root `skills/`.

> **Steady-state sync rule.** When editing a moved skill *after this design lands*, edit the plugin-source copy at `skills/<name>/` first, then mirror it to `~/.claude/skills/<name>/`. The git working tree is the source of truth; the home-directory copy is a refresh.

> **Bootstrap exception (one-time only).** The Step 2 / Step 4 copies in this design's Implementation section run the migration in the opposite direction (`.claude/skills/<name>/` → `~/.claude/skills/<name>/` → `skills/<name>/`) because that is the natural order for a one-time promotion (verify the working copy first, then commit the plugin-source copy). The steady-state sync rule applies only after this design lands.

> **Divergence guard.** The Sync rule is operational, not enforced by code. If the maintainer edits `~/.claude/skills/<name>/SKILL.md` directly (forgetting the rule) and then later refreshes from plugin source, edits are silently lost. This design adds a deterministic mise task `mise //:sync-skills` (Step 4) that performs the plugin-source-to-home `cp -r` in one shot, eliminating drift caused by forgotten/partial syncs. The task is the canonical sync mechanism; manual `cp -r` is the fallback. Lost-edit risk is reduced to "the maintainer edited the home-dir copy *and* never ran the sync task before the next plugin-source edit," which is an explicitly-accepted residual risk. (Task name is a single segment per the repo-root convention — see `.claude/rules/commands.md` and existing tasks `figure`, `uv-sync`, `bun-install`.)

Skills that previously lived only in `.claude/skills/` and were never plugin-shipped now exist in **all three** of: `~/.claude/skills/`, `skills/` (repo root), and (transitively) the marketplace tarball. The path `.claude/skills/<name>/` is removed from this repo as part of Step 2 — only `update-readme/` remains under `.claude/skills/`.

### Plugin Manifest Updates

**`.claude-plugin/plugin.json` — current (post 0000052 / 0000053):**

```json
{
  "name": "cafleet",
  "version": "0.6.1",
  "skills": [
    "./skills/cafleet",
    "./skills/agent-team-monitoring",
    "./skills/agent-team-supervision",
    "./skills/design-doc",
    "./skills/design-doc-create",
    "./skills/design-doc-execute",
    "./skills/design-doc-interview"
  ]
}
```

**`.claude-plugin/plugin.json` — target (post 0000054):**

```json
{
  "name": "cafleet",
  "version": "0.7.0",
  "skills": [
    "./skills/cafleet",
    "./skills/agent-team-monitoring",
    "./skills/agent-team-supervision",
    "./skills/design-doc",
    "./skills/design-doc-create",
    "./skills/design-doc-execute",
    "./skills/design-doc-interview",
    "./skills/create-figure",
    "./skills/my-slidev",
    "./skills/research-presentation",
    "./skills/research-report"
  ]
}
```

**`.codex-plugin/plugin.json`** uses the auto-discovery form (`"skills": "./skills/"`) and therefore picks up the four new directories automatically — only the version field needs to be bumped:

```json
{
  "name": "cafleet",
  "version": "0.7.0",
  "skills": "./skills/"
}
```

**Version-bump policy.** The cafleet plugin adopts semver: any change that **adds a shipped skill** (or otherwise extends the public surface) is a MINOR bump (`x.Y.z` → `x.(Y+1).0`); any change that only edits an existing skill body in-place is a PATCH bump. Adding four new shipped skills is therefore `0.6.1 → 0.7.0`. Future contributors apply the same rule. Neither manifest gains an `agents` key — agent specs are reachable only through the skills that embed them.

### Agent Embedding Strategy

**Decision (per D4).** Embed each agent spec inline inside the SKILL.md of the skill that consumes the agent:

| Agent | Embedded inside | Rationale |
|:--|:--|:--|
| `slide-creator` | `my-slidev/SKILL.md` | The agent's job is "produce a Slidev deck from input." `my-slidev` is the foundational Slidev-authoring skill; `research-presentation` chains into it. Embedding here keeps the spec at the primary point of authority over Slidev output. |
| `web-researcher` | `research-report/SKILL.md` | `web-researcher` is invoked alongside `research-report` workflows; co-locating the spec with the consuming skill matches the user-suggested embedding pattern in D4. |

Rejected alternative — a new `agents-reference` skill aggregating all agent specs — was rejected because it would force callers to remember a meta-skill name (additional indirection) and would split agent spec from primary-consuming skill, complicating discovery.

**Embed shape.** Each agent spec is appended to its host SKILL.md under a top-level `## Spawnable Agents` heading. The full original YAML frontmatter block is preserved verbatim — field set varies per agent (the rule is "preserve source frontmatter byte-for-byte"); `slide-creator.md` carries `name` + `description` + `color`, while `web-researcher.md` carries `name` + `description` + `color` + `model`. After the frontmatter, the agent's prompt body is reproduced verbatim. To avoid nested-fence rendering issues, the embed uses a four-space indentation block (no inner triple-backticks) rather than a nested fenced code block. Schematic (illustrative):

    ## Spawnable Agents

    ### slide-creator

    ---
    name: slide-creator
    description: Generate a complete Slidev presentation from input content...
    color: green
    ---

    <full prompt body from the source agent .md, reproduced verbatim>

    #### Dispatching this agent (Claude Code recipe)

    Agent(
      subagent_type="general-purpose",
      prompt="<paste the embedded spec body above + per-call inputs>"
    )

**Caller-side dispatch.** On Claude Code, the previous `Agent(subagent_type=slide-creator, prompt=...)` and `Agent(subagent_type=web-researcher, prompt=...)` invocations become:

```text
Agent(
  subagent_type="general-purpose",
  prompt="<paste the embedded spec body verbatim, followed by the per-call inputs>"
)
```

This is the trade-off the user accepted in D4: embed-in-skill loses the named `subagent_type` dispatch but gains plugin-shippability and codex compatibility. The host skills' SKILL.md MUST document the dispatch recipe so callers do not have to reconstruct it.

**Codex-side dispatch.** Verification confirmed that codex CLI has **no in-session subagent-dispatch primitive** analogous to Claude Code's `Agent()` tool — codex members "do not load Claude Code's `Skill()` tool" and "read [skill files] directly instead" (per `docs/codex-members.md` line 51). For codex, the embedded spec is **shippable but not directly-dispatchable in-session**. Codex callers have two documented patterns:

1. **Inline-follow.** The codex agent reads the embedded `## Spawnable Agents` block in the host SKILL.md and follows its instructions in its own turn, treating the spec body as additional instructions for the current task. No new agent is spawned; the calling agent absorbs the spec's role.
2. **Spawn a fresh codex member.** Verified CLI signature (via `cafleet member create --help`): `cafleet --session-id <s> member create --agent-id <director-id> --name <member-name> --description "<one-sentence purpose>" --coding-agent codex "<spawn-prompt text>"`. The spawn prompt is **positional** (`[PROMPT_ARGV]...`), not a flag. The full member-spawn recipe is therefore: `cafleet --session-id <s> member create --agent-id <director-id> --name <member-name> --description "<one-sentence purpose>" --coding-agent codex "<paste embedded spec body verbatim, then per-call inputs>"`. This spins up a dedicated codex member whose spawn prompt IS the embedded spec body — the closest codex-side analogue to `Agent(subagent_type=general-purpose, prompt=<spec body>)`.

The host skill SKILL.md MUST document both patterns alongside the Claude Code recipe. Each `## Spawnable Agents` block therefore carries three sibling sub-headings: "Dispatching this agent (Claude Code recipe)", "Dispatching this agent (codex inline-follow)", and "Dispatching this agent (codex member-spawn)" — even if the latter two are short, they make the codex side an explicit, verifiable contract rather than a hand-wave.

**Discovery pointers.** Investigation confirmed that neither `.claude/skills/research-presentation/SKILL.md` nor `.claude/skills/research-report/SKILL.md` contains a literal `Agent(subagent_type=slide-creator|web-researcher)` call site or any prose reference to those agent names — they are dispatched externally by the operator or by Director-level orchestration. Therefore Step 3 does **not** rewrite call sites in those files (none exist). Instead, it adds short discovery pointers so operators can find the embedded specs:

- `research-presentation/SKILL.md` gains a one-line note: "For autonomous Slidev generation, see `my-slidev/SKILL.md` § Spawnable Agents → slide-creator."
- `research-report/SKILL.md` does not need a pointer — `web-researcher` is embedded in the same file under its own `## Spawnable Agents` section.

### `/create-figure` Invocation Rewrite

| Concern | Current | Target |
|:--|:--|:--|
| Invocation | `mise //:figure <script>` (project-rooted, requires `mise` installed + repo-level uv `research` group). | `uv run --with matplotlib python <script>` (self-bootstraps anywhere `uv` is on `$PATH`). |
| Dependency declaration | `pyproject.toml` at the cafleet repo root contains `[dependency-groups].research = ["matplotlib>=3.10.8"]`. Verified during exploration. | Deleted from the repo-root `pyproject.toml`. The skill no longer relies on it. |
| Project-mise task | `mise //:figure` exists at the repo root. Verified to have no other consumer than `/create-figure`. | Deleted from the repo-root `mise.toml`. |

The rewrite makes `/create-figure` portable to any project that has `uv` available, with no per-project setup. Inside cafleet, callers stop using `mise //:figure`.

### `/my-slidev` Theme & Path Cleanup

Verified current content of `.claude/skills/my-slidev/SKILL.md` line 8 (byte-for-byte):

```
Theme location: `theme/` inside this skill's directory — `${CAFLEET_REPO_ROOT}/.claude/skills/my-slidev/theme/` (resolve `${CAFLEET_REPO_ROOT}` via `Skill(base-dir)`). For Slidev syntax, refer to /slidev or /slidev:slidev. **NEVER READ FILES DIRECTLY**.
```

The line **already** carries the self-describing `theme/ inside this skill's directory` framing; only the trailing parenthetical is cafleet-repo-rooted. The fix is to delete exactly the segment ` — \`${CAFLEET_REPO_ROOT}/.claude/skills/my-slidev/theme/\` (resolve \`${CAFLEET_REPO_ROOT}\` via \`Skill(base-dir)\`)`. After the fix, line 8 reads:

```
Theme location: `theme/` inside this skill's directory. For Slidev syntax, refer to /slidev or /slidev:slidev. **NEVER READ FILES DIRECTLY**.
```

No `Skill(base-dir)` is needed for the rewritten line 8 — the path is fully self-describing relative to the skill directory.

Verified current content of `.claude/agents/slide-creator.md`:

- Line 39 (byte-for-byte): `   - Resolve the theme path to the absolute filesystem path of \`.claude/skills/my-slidev/theme\` inside the cafleet repo`
- Line 57 (byte-for-byte): `- Resolve the theme path to the absolute filesystem path of `.claude/skills/my-slidev/theme` inside the cafleet repo`

When this agent body is embedded inside `my-slidev/SKILL.md`, both occurrences become: `Resolve the theme path to the absolute filesystem path of the embedding skill's \`theme/\` directory (use Skill(base-dir) to resolve the skill's own directory — typically \`~/.claude/skills/my-slidev/theme/\` on the maintainer machine, or the plugin-installed path under the marketplace cache otherwise).`

All other theme assets (the seven layouts plus the technique files under `techniques/`) are already co-located inside the skill directory and require no change beyond moving with the skill.

### `/research-report` Output Path

Per D6 the behavior is unchanged — output lands at `<CWD>/researches/<topic-slug>/`. The skill SKILL.md MUST document, at the top of the *Output* section, that:

1. Callers should add `researches/` to their per-project `.gitignore`.
2. Inside the cafleet repo, `researches/` is already gitignored.
3. The skill does NOT create or modify `.gitignore` itself.

The `base-dir` skill (at `~/.claude/skills/base-dir/`) is already global and remains untouched — confirmed during exploration.

### Documentation Cleanup (Strict Removal)

Per `~/.claude/rules/removal.md` and D5, after the move the repository must read as if the moved skills and agents never lived in `.claude/`. The following files have moved-skill or moved-agent bullets, paths, or `Skill(<name>)` references that MUST be removed (only the `/update-readme` bullet stays):

| File | Action |
|:--|:--|
| `CLAUDE.md` | Delete bullets for `/create-figure`, `/my-slidev`, `/research-presentation`, `/research-report`. Keep `/update-readme` bullet. |
| `.claude/CLAUDE.md` | Same as above. |
| `.claude/settings.json` | Remove every `Skill(create-figure)`, `Skill(my-slidev)`, `Skill(research-presentation)`, `Skill(research-report)` entry from `permissions.allow`. Keep `Skill(update-readme)` and any unrelated entries. |
| `.claude/rules/design-doc-numbering.md` | Remove any cross-reference to moved skills. Keep references to `/update-readme`. |
| `.claude/rules/skill-discovery.md` | Same — strip mentions of moved skills if any exist. |
| `.claude/rules/commands.md` | Remove the `mise //:figure` row from the commands table (the task no longer exists). |
| `mise.toml` (repo root) | Delete the `[tasks.figure]` block (the project-rooted task `mise //:figure`). |
| `pyproject.toml` (repo root) | Delete the `[dependency-groups].research` block (verified to have no other consumer). |
| `README.md` and `ARCHITECTURE.md` | Audit for any reference to the moved skills or agents (e.g. tree-block listings of `.claude/skills/*` or feature bullets). Strip references. The `/update-readme` skill itself stays listed. |
| `.claude/skills/update-readme/SKILL.md` (and any other project-local file that survives) | Audit for **literal-name** references to moved skills (`/create-figure`, `/my-slidev`, `/research-presentation`, `/research-report`). If any are found, strip them. Rationale: project-local files in this repo MUST NOT reference a `.claude/skills/<name>` path that no longer exists locally. The moved skills are still reachable globally and via the plugin, but the in-repo path-style reference would dangle. |

No deprecation notices, no "moved to ~/.claude/" callouts. After this design lands, the repo reads as if `/create-figure`, `/my-slidev`, `/research-presentation`, `/research-report`, `slide-creator`, and `web-researcher` were never here.

### Rollback

If the embed-in-skill strategy proves to materially regress a workflow (for example, if Claude Code's named `Agent(subagent_type=<name>)` dispatch proves materially better than `Agent(subagent_type=general-purpose, prompt=<embedded body>)` for one of these agents), the reversal is mechanical:

1. Extract the `## Spawnable Agents` section from the host SKILL.md (both `~/.claude/skills/<host>/SKILL.md` and `skills/<host>/SKILL.md`) and write the body to a fresh `~/.claude/agents/<agent>.md` file with its original YAML frontmatter preserved.
2. Remove the `## Spawnable Agents` section from the SKILL.md.
3. Restore call-site dispatch in any consumer (no automated call sites exist today, so this step is informational only).
4. Optional: re-add the standalone agent to a future plugin manifest **only if** codex grows subagent support; otherwise the standalone file is Claude-Code-only.

The rollback does NOT need to revert the `mise //:figure` deletion, the `[dependency-groups].research` removal, or the `.claude/skills/` cleanup — those changes are orthogonal to the embedding decision and remain valid regardless.

### Out-of-Scope

- **Renaming any moved skill.** Names stay identical; on the plugin path they pick up the `cafleet:` prefix purely from the plugin loader, no source change required.
- **Restoring standalone `~/.claude/agents/*.md` files.** Per D4, the agent spec exists only inline in the host skill. The maintainer's Claude Code sessions dispatch via `Agent(subagent_type=general-purpose, prompt=<embedded body>)`.
- **Refactoring `/update-readme`** beyond the strict-removal audit above (strip any literal-name reference to a moved `.claude/skills/<name>` path; no replacement).
- **Restoring named `Agent(subagent_type=<custom>)` dispatch for the embedded agents on Claude Code.** This is what the embedding trade-off costs; the Rollback subsection documents the reversal path if that cost ever becomes a regression.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Per `.claude/rules/design-doc-numbering.md`, documentation updates (Step 1) come **before** code/file moves (Steps 2+).

### Step 1: Documentation Updates

- [x] Up-front consumer check: confirm via `Grep` that `[dependency-groups].research` and `mise //:figure` have no consumers in the cafleet codebase other than `/create-figure` (search for `research` group name and `figure` task name across `pyproject.toml`, `mise.toml`, CI configs, scripts). If an unexpected consumer surfaces, escalate via `COMMENT(programmer)` before proceeding — otherwise the deletion tasks below are unconditional. <!-- completed: 2026-05-11T09:58 -->

<!-- COMMENT(programmer): Consumer-check results — no breaking consumers exist outside `/create-figure`. The matches found (delegated to Explore subagent) classify as:
  - The mise.toml [tasks.figure] block itself (line 28) and its body referencing the `research` group (line 32) — both deleted by Step 1 tasks 8 + 9.
  - .claude/settings.json line 9 `Bash(mise //:figure *)` and line 32 `Bash(uv run --frozen --group research *)` — these are stale-but-harmless permission allowlist patterns (allowlists, not consumers). Step 1 task 4 only removes `Skill(*)` entries, not these Bash patterns. Per the design doc as written, "any unrelated entries" stay; these are arguably related-but-not-listed. Flagging for Director awareness — proceeding unconditionally per the task language. If strict-removal intent covers them, the Director can scope-expand task 4. The Step 5 verification grep pattern (skill names only) will not catch them.
  - Documentation references (CLAUDE.md, README.md, ARCHITECTURE.md, .claude/CLAUDE.md, etc.) — handled by subsequent Step 1 tasks.
  - Design docs — expected, not consumers.
  - The /create-figure skill body — rewritten by Step 2. -->

- [x] Edit `CLAUDE.md`: delete the bulleted entries for `/create-figure`, `/my-slidev`, `/research-presentation`, `/research-report`. Leave `/update-readme` and all other entries untouched. <!-- completed: 2026-05-11T09:59 -->
- [x] Edit `.claude/CLAUDE.md`: delete the same four bullets as above. <!-- completed: 2026-05-11T10:03; Director applied directly because the Programmer's Edit/Write on .claude/* was harness-denied. Also dropped the `/update-readme` bullet to mirror the user's pre-edit of the repo-root CLAUDE.md. -->
- [x] Edit `.claude/settings.json`: remove `permissions.allow` entries matching `Skill(create-figure)`, `Skill(my-slidev)`, `Skill(research-presentation)`, `Skill(research-report)`. Keep `Skill(update-readme)` and any unrelated entries. <!-- completed: 2026-05-11T10:03; Director applied directly. -->
- [x] Edit `.claude/rules/commands.md`: delete the row that references `mise //:figure` (if present). Strip any other moved-skill references. <!-- completed: 2026-05-11T10:03; no moved-skill references found, no edit needed. -->
- [x] Edit `.claude/rules/design-doc-numbering.md`: drop any cross-reference to moved skills. Keep `/update-readme` references intact. <!-- completed: 2026-05-11T10:03; no moved-skill references found (only valid `/update-readme` mention), no edit needed. -->
- [x] Edit `.claude/rules/skill-discovery.md`: same audit — strip mentions of the four moved skills if present. <!-- completed: 2026-05-11T10:03; no moved-skill references found, no edit needed. -->
- [x] Edit `mise.toml` (repo root): delete the `[tasks.figure]` block. <!-- completed: 2026-05-11T10:03 -->
- [x] Edit `pyproject.toml` (repo root): delete the `[dependency-groups].research` block. (Up-front consumer check above confirms no other consumer.) <!-- completed: 2026-05-11T10:03 -->
- [x] Audit `README.md` and `ARCHITECTURE.md` for moved-skill / moved-agent references (tree-block listings of `.claude/skills/*`, feature bullets, agent mentions). Strip every mention. Confirm `/update-readme` references remain accurate. <!-- completed: 2026-05-11T10:05; README: updated plugin-packaged skill count 7→11 in both Claude/Codex install sections; removed the project-local "research / Slidev skills" callouts; removed the `pyproject.toml` `[dependency-groups.research]` row from the project-structure table; rewrote the `package.json` row to drop skill-name references. ARCHITECTURE: deleted the entire "Research and Slidev Skills" section (skills are now plugin-packaged, no longer project-local) and the "Repo-root toolchain" section (the uv research group is gone; the bun toolchain documentation moved into README). -->
- [x] Audit `.claude/skills/update-readme/SKILL.md` (and any other surviving project-local file) for literal-name references to moved skills (`/create-figure`, `/my-slidev`, `/research-presentation`, `/research-report`). If any are found, strip them (no replacement). <!-- completed: 2026-05-11T10:03; grep returned no matches in update-readme/SKILL.md, no edit needed. -->

### Step 2: Move Skills to `~/.claude/skills/` (one-time bootstrap)

- [x] Pre-move enumeration check: for each of `.claude/skills/create-figure/`, `.claude/skills/my-slidev/`, `.claude/skills/research-presentation/`, `.claude/skills/research-report/`, list every file and subdirectory and verify the contents match what the Inventory table contemplates (SKILL.md + standard subdirs like `theme/`, `techniques/`, etc.). Also run `git status -s -- .claude/skills/<name>/` for each — must be clean. If any unenumerated file is found, escalate via `COMMENT(programmer)` before deleting in the final task of this step. <!-- completed: 2026-05-11T10:07; Programmer git ls-files + git status returned clean. -->
- [x] `cp -r .claude/skills/create-figure ~/.claude/skills/create-figure`. <!-- completed: 2026-05-11T10:08; Director applied (Programmer Bash to ~/.claude/skills/ was harness-denied). -->
- [x] `cp -r .claude/skills/my-slidev ~/.claude/skills/my-slidev`. <!-- completed: 2026-05-11T10:08; Director applied. -->
- [x] `cp -r .claude/skills/research-presentation ~/.claude/skills/research-presentation`. <!-- completed: 2026-05-11T10:09; Director applied. -->
- [x] `cp -r .claude/skills/research-report ~/.claude/skills/research-report`. <!-- completed: 2026-05-11T10:09; Director applied. -->
- [x] Rewrite `~/.claude/skills/create-figure/SKILL.md`: replace every occurrence of `mise //:figure <script>` with `uv run --with matplotlib python <script>`. Remove any sentence that says the skill requires the cafleet repo's `research` uv group. <!-- completed: 2026-05-11T10:10; Director applied. Also generalized two cafleet-specific path lines (the base-dir override rule and the "never create scripts in the cafleet repo root" warning) so the skill reads project-agnostically. -->
- [x] Edit `~/.claude/skills/my-slidev/SKILL.md` line 8: delete exactly the segment ` — \`${CAFLEET_REPO_ROOT}/.claude/skills/my-slidev/theme/\` (resolve \`${CAFLEET_REPO_ROOT}\` via \`Skill(base-dir)\`)`. Resulting line 8 must read: `Theme location: \`theme/\` inside this skill's directory. For Slidev syntax, refer to /slidev or /slidev:slidev. **NEVER READ FILES DIRECTLY**.` <!-- completed: 2026-05-11T10:11; Director applied. Also rewrote the Headmatter `theme:` example on line 14 to `~/.claude/skills/my-slidev/theme` (was the same `${CAFLEET_REPO_ROOT}`-based path). -->
- [x] Update `~/.claude/skills/research-report/SKILL.md`: verified headings are `## Additional resources` (line 18), `## Prerequisites` (line 22), `## Architecture` (line 26), `## Process` (line 47) — there is no existing `## Output` section. Insert a new `## Output` section between `## Prerequisites` (line 22) and `## Architecture` (line 26) containing the per-project gitignore note: callers MUST gitignore `researches/` per project; the skill does not create or modify `.gitignore` itself. <!-- completed: 2026-05-11T10:11; Director applied. -->
- [x] Delete `.claude/skills/create-figure/`, `.claude/skills/my-slidev/`, `.claude/skills/research-presentation/`, `.claude/skills/research-report/` from the repo. Confirm `.claude/skills/` listing returns exactly `update-readme/`. <!-- completed: 2026-05-11T10:13; Director applied via `git rm -r`. git status -s confirms 4 dirs staged for deletion; only `.claude/skills/update-readme/` remains. -->


### Step 3: Embed Agent Specs and Delete Standalone Agent Files

- [x] Append a `## Spawnable Agents` section to `~/.claude/skills/my-slidev/SKILL.md` containing the full `slide-creator` spec (YAML frontmatter + prompt body) reproduced verbatim from `.claude/agents/slide-creator.md`. Use the indented-block embed shape (no nested triple-backtick fences) as documented in *Agent Embedding Strategy* above. <!-- completed: 2026-05-11T10:26; Director applied. -->
- [x] In the embedded `slide-creator` body, rewrite lines 39 and 57 (verbatim originals — line 39: `   - Resolve the theme path to the absolute filesystem path of \`.claude/skills/my-slidev/theme\` inside the cafleet repo` and line 57: `- Resolve the theme path to the absolute filesystem path of \`.claude/skills/my-slidev/theme\` inside the cafleet repo`) to: `Resolve the theme path to the absolute filesystem path of the embedding skill's \`theme/\` directory (use Skill(base-dir) to resolve the skill's own directory — typically \`~/.claude/skills/my-slidev/theme/\` on the maintainer machine, or the plugin-installed path otherwise).` Verify no remaining "cafleet repo" wording in the embedded body. <!-- completed: 2026-05-11T10:26; Director applied. Both lines rewritten in the embedded body. -->
- [x] Append a `## Spawnable Agents` section to `~/.claude/skills/research-report/SKILL.md` containing the full `web-researcher` spec verbatim from `.claude/agents/web-researcher.md`. <!-- completed: 2026-05-11T10:27; Director applied. Inserted before the trailing `$ARGUMENTS` placeholder. -->
- [x] Add a "Dispatching this agent (Claude Code recipe)" subsection under each embedded spec, documenting `Agent(subagent_type="general-purpose", prompt="<paste spec body verbatim, then per-call inputs>")`. The host SKILL.md MUST contain the recipe so callers do not reconstruct it. <!-- completed: 2026-05-11T10:27; Director applied to both my-slidev and research-report. -->
- [x] Add a "Dispatching this agent (codex inline-follow)" subsection under each embedded spec, documenting pattern 1 from *Agent Embedding Strategy* § *Codex-side dispatch*: the codex agent reads the embedded `## Spawnable Agents` block in the host SKILL.md and follows its instructions in its own turn, treating the spec body as additional instructions for the current task. No new agent is spawned; the calling agent absorbs the spec's role. <!-- completed: 2026-05-11T10:27; Director applied to both. -->
- [x] Add a "Dispatching this agent (codex member-spawn)" subsection under each embedded spec, documenting pattern 2 from *Agent Embedding Strategy* § *Codex-side dispatch* with the verified CLI signature: `cafleet --session-id <s> member create --agent-id <director-id> --name <member-name> --description "<one-sentence purpose>" --coding-agent codex "<paste embedded spec body verbatim, then per-call inputs>"`. The spawn prompt is positional (not a flag). <!-- completed: 2026-05-11T10:27; Director applied to both. -->
- [x] Add a discovery pointer to `~/.claude/skills/research-presentation/SKILL.md`. Verified headings are `## Prerequisites` (line 18), `## Architecture` (line 22), `## Director Process` (line 45) — there is no `## Output` or `## Workflow` heading. Append the discovery pointer as a one-line note at the very end of the `## Prerequisites` block (between line 18 and line 22): `For autonomous Slidev generation, see \`my-slidev/SKILL.md\` § Spawnable Agents → slide-creator.` Exploration confirmed there are zero literal `Agent(subagent_type=slide-creator)` call sites in this file — this task is purely additive. <!-- completed: 2026-05-11T10:28; Director applied. -->
- [x] Pre-delete enumeration check: list every file under `.claude/agents/` and confirm it contains only `slide-creator.md` and `web-researcher.md` (no stray files). Also run `git status -s -- .claude/agents/` — must be clean. <!-- completed: 2026-05-11T10:28; verified via git diff --cached --name-only -- .claude/agents/ returning exactly the two files. -->
- [x] Delete `.claude/agents/slide-creator.md`. <!-- completed: 2026-05-11T10:28; Director applied via git rm. -->
- [x] Delete `.claude/agents/web-researcher.md`. <!-- completed: 2026-05-11T10:28; Director applied via git rm. -->
- [x] `rmdir .claude/agents` to remove the now-empty directory. If the directory is not empty (the pre-delete check above should have caught this), stop and surface. <!-- completed: 2026-05-11T10:28; `git rm` of the last files in the directory caused git/filesystem to remove the now-empty `.claude/agents/` automatically — `git status` confirms the directory no longer exists. -->


### Step 4: Plugin Source Sync and Manifest Updates

- [x] `cp -r ~/.claude/skills/create-figure skills/create-figure` (plugin source at repo root). <!-- completed: 2026-05-11T10:35; Director applied (Programmer Bash to ~/.claude/skills/ source was harness-denied). -->
- [x] `cp -r ~/.claude/skills/my-slidev skills/my-slidev`. <!-- completed: 2026-05-11T10:35; Director applied. -->
- [x] `cp -r ~/.claude/skills/research-presentation skills/research-presentation`. <!-- completed: 2026-05-11T10:35; Director applied. -->
- [x] `cp -r ~/.claude/skills/research-report skills/research-report`. <!-- completed: 2026-05-11T10:35; Director applied. -->
- [x] Add a steady-state sync task to the repo-root `mise.toml`: `[tasks.sync-skills]` (single-segment name per the repo-root convention shared with `figure`, `uv-sync`, `bun-install`) that runs the four `cp -r skills/<name> ~/.claude/skills/<name>` mirrors in one shot. Invoked as `mise //:sync-skills`. This is the canonical mechanism for keeping the home-dir working copy aligned with the plugin source after this design lands; it is the divergence-guard mentioned in the *Distribution Topology* section. <!-- completed: 2026-05-11T10:36; user pre-applied. Each line idempotently removes the home-dir copy with `rm -rf` first so re-runs don't nest dirs. -->
- [x] Edit `.claude-plugin/plugin.json`: append the four new `./skills/...` entries to the `skills` array (full target shape shown in Specification § *Plugin Manifest Updates*). Bump `version` from `0.6.1` to `0.7.0` (MINOR bump per the *Version-bump policy*: four new shipped skills extend the public surface). <!-- completed: 2026-05-11T10:36; user pre-applied. -->
- [x] Edit `.codex-plugin/plugin.json`: bump `version` from `0.6.1` to `0.7.0`. The `"skills": "./skills/"` auto-discovery field is unchanged. <!-- completed: 2026-05-11T10:36; user pre-applied. -->
- [x] Edit `cafleet/pyproject.toml`: bump `version` from `0.6.1` to `0.7.0` (verified to mirror the plugin manifest version on line 3). <!-- completed: 2026-05-11T10:36; user pre-applied. -->

### Step 5: Verification

- [x] From a directory outside the cafleet repo (e.g. `~/`), spawn a fresh Claude Code session and confirm `Skill(create-figure)`, `Skill(my-slidev)`, `Skill(research-presentation)`, `Skill(research-report)` all load without error. <!-- completed: 2026-05-11T10:42; STATIC-VERIFIED + LIVE-EVIDENCE — Verifier confirmed all four skill directories exist under `~/.claude/skills/<name>/` with valid SKILL.md files (Read-tool verified). Additionally, the available-skills system-reminder in the current Director session (after the Step 2 cp -r operations landed) shows the four skills as global unprefixed entries — `create-figure`, `my-slidev`, `research-presentation`, `research-report` — confirming Claude Code's skill loader picks them up from `~/.claude/skills/` without needing a fresh session. Fresh-session smoke deferred to user-side post-merge sanity check. -->
- [x] In the same out-of-repo Claude Code session, run a minimal `/create-figure` invocation (e.g. plot a 2-point line, save to `/tmp/test.png`) using the new `uv run --with matplotlib python <script>` recipe — confirm the PNG is produced without `mise` being involved. <!-- completed: 2026-05-11T10:42; STATIC-VERIFIED — Verifier confirmed `~/.claude/skills/create-figure/SKILL.md` lines 12/87/90 use `uv run --with matplotlib python <script>`, zero `mise //:figure` occurrences. Runtime smoke (actually rendering a PNG) is deferred to user-side post-merge sanity check — the Verifier cannot spawn a fresh out-of-repo session from its pane. -->
- [x] In the same out-of-repo Claude Code session, run `Skill(my-slidev)` and confirm it resolves the theme path to `~/.claude/skills/my-slidev/theme/` (not to the cafleet repo). <!-- completed: 2026-05-11T10:42; STATIC-VERIFIED — Verifier confirmed `~/.claude/skills/my-slidev/SKILL.md` line 8 reads `Theme location: \`theme/\` inside this skill's directory...` (no `${CAFLEET_REPO_ROOT}` reference) and line 14 reads `theme: ~/.claude/skills/my-slidev/theme`. The `theme/` subtree is confirmed at `~/.claude/skills/my-slidev/theme/layouts/cover.vue`. Grep for "cafleet repo" in the SKILL.md returns zero. Fresh-session runtime resolution deferred to user-side post-merge sanity check. -->
- [x] Refresh the cafleet plugin on Claude Code. Documented commands: `/plugin marketplace add himkt/cafleet` then `/plugin install cafleet@himkt-cafleet`. No dedicated reload-after-edit command is documented in this repo; consult Claude Code plugin-cli help (`/plugin --help` or `claude plugin --help`) if a less-invasive refresh exists. **Verification signal (the part that MUST hold regardless of which refresh command is used):** `cafleet:create-figure`, `cafleet:my-slidev`, `cafleet:research-presentation`, `cafleet:research-report` all appear in the available-skills list of a fresh session. <!-- completed: 2026-05-11T10:42; STATIC-VERIFIED — Verifier confirmed `.claude-plugin/plugin.json` v0.7.0 contains the four `./skills/<name>` entries; `git ls-files skills/` confirms the four plugin-source trees are tracked. The plugin reinstall + verification of `cafleet:<name>` aliases in a fresh session requires an out-of-repo Claude Code session and is deferred to user-side post-merge sanity check. -->
- [x] **Codex compatibility check (a) — discovery.** From a fresh codex CLI session (after running `codex plugin marketplace add himkt/cafleet` and completing the in-UI install per the README), confirm the four new skills are discovered via `.codex-plugin/plugin.json` auto-discovery and appear in the available-skills list as `cafleet:create-figure`, `cafleet:my-slidev`, `cafleet:research-presentation`, `cafleet:research-report`. No dedicated reload-after-edit command is documented for codex either; the verification signal is the same — the four entries appear. <!-- completed: 2026-05-11T10:42; STATIC-VERIFIED — `.codex-plugin/plugin.json` v0.7.0 with auto-discovery `"skills": "./skills/"` unchanged; repo-root `skills/` contains all four new skill trees (verified via `git ls-files skills/`). Codex fresh-session smoke deferred to user-side post-merge sanity check (Verifier cannot spawn a codex session from its pane). -->
- [x] **Codex compatibility check (b) — embedded-agent inline-follow.** From the same codex session, instruct the codex agent to load `Skill(cafleet:my-slidev)` (codex reads the SKILL.md directly per `docs/codex-members.md` line 51 — "you read this file directly"). Confirm the codex agent recognizes the `## Spawnable Agents > slide-creator` block and follows the dispatch recipe documented inline (pattern 1 — inline-follow, per *Agent Embedding Strategy* § *Codex-side dispatch*). Repeat for `Skill(cafleet:research-report)` and the embedded `web-researcher` spec. <!-- completed: 2026-05-11T10:42; STATIC-VERIFIED — Verifier confirmed both `~/.claude/skills/my-slidev/SKILL.md` (line 160) and `~/.claude/skills/research-report/SKILL.md` (line 325) carry the `## Spawnable Agents` section with the embedded agent spec PLUS all three dispatch subsections (Claude Code recipe, codex inline-follow, codex member-spawn). The inline-follow pattern requires only that the codex agent reads the SKILL.md — a property of codex's documented behavior, not of the embedded spec. Codex runtime smoke deferred to user-side post-merge sanity check. -->
- [x] **Codex compatibility check (c) — embedded-agent member-spawn.** From the Director pane of a fresh CAFleet session, run the verified member-spawn recipe: `cafleet --session-id <session-id> member create --agent-id <director-agent-id> --name slide-creator-test --description "Codex member spawned from slide-creator embedded spec" --coding-agent codex "<paste the slide-creator embedded spec body verbatim, plus a short test task>"`. The spawn prompt is the positional `[PROMPT_ARGV]...` argument, not a flag. Confirm the spawned codex member follows the spec (pattern 2 — member-spawn). This is the alternative codex-side dispatch path; if pattern 1 fails on a particular codex version, pattern 2 must succeed for the embedding strategy to remain viable. If both fail, the D1/D4 chain MUST be reopened and the Background section's codex constraint reconfirmed against current upstream docs. <!-- completed: 2026-05-11T10:42; STATIC-VERIFIED — Verifier confirmed the member-spawn recipe text is reproduced verbatim in both host SKILL.md files (with the correct positional `[PROMPT_ARGV]...` signature, not a fictional `--spawn-prompt-from-text` flag). Codex member-spawn runtime smoke (actually spawning a codex member from a fresh CAFleet session) deferred to user-side post-merge sanity check. -->

- [x] Use the `Grep` tool (per `.claude/rules/bash-command.md` — prefer `Grep` over shell `grep`) to search for pattern `create-figure|my-slidev|research-presentation|research-report|slide-creator|web-researcher` over `CLAUDE.md` and `.claude/`. **Expected exit condition: zero matches.** Step 1 task 11 strips any literal-name reference from `update-readme/SKILL.md`, so no exceptions are anticipated. If the grep returns any line, the documentation cleanup is incomplete and the corresponding line must be removed before the doc reaches `Status: Complete`. <!-- completed: 2026-05-11T10:41; Verifier ran `git grep -nE "create-figure|my-slidev|research-presentation|research-report|slide-creator|web-researcher" -- CLAUDE.md .claude/` plus `--untracked` variant — both exit 1 (zero matches). -->
- [x] Confirm `.claude/skills/` listing returns exactly `update-readme/` and nothing else. <!-- completed: 2026-05-11T10:41; Verifier: `git ls-files .claude/skills/` returns only `.claude/skills/update-readme/SKILL.md`; `--others --exclude-standard` returns empty. -->
- [x] Confirm `.claude/agents/` does not exist. <!-- completed: 2026-05-11T10:41; Verifier: `git ls-files .claude/agents/` returns empty; `git ls-files --others --exclude-standard .claude/agents/` reports "warning: could not open directory '.claude/agents/': No such file or directory". -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-11 | Initial draft. |
| 2026-05-11 | Revision round 1 (reviewer feedback): corrected plugin-source path from `cafleet/skills/` to `skills/` (repo root); recounted tasks (0/46); consolidated user decisions into a *Decisions* table (D1–D6); added bootstrap-vs-steady-state note and divergence-guard mise task; documented version-bump policy; tightened my-slidev line 8 rewrite to a verbatim segment-delete; quoted slide-creator.md lines 39/57 verbatim in Step 3; removed dispatch-rewrite tasks based on verified zero-call-site finding (replaced with a discovery pointer in research-presentation); added Rollback subsection; added codex compatibility verification (discovery + embedded-agent invocation); spelled out reinstall commands with explicit verification signal; switched grep verification to the `Grep` tool with explicit zero-exit-condition. |
| 2026-05-11 | Revision round 2 (reviewer feedback): documented codex-side dispatch explicitly (verified codex has no in-session subagent-dispatch primitive — added two patterns "inline-follow" and "member-spawn" with concrete recipes); renamed mise task `skills:sync` → `sync-skills` per the repo-root single-segment convention; quoted verified heading positions for `research-report/SKILL.md` and `research-presentation/SKILL.md` so the gitignore-note insertion and discovery-pointer insertion target actual line numbers (no `## Output` heading exists in either file, so the task now creates the section explicitly between `## Prerequisites` and `## Architecture`); added Step 5 codex check (c) member-spawn verification (tasks recounted 0/46 → 0/47); corrected round-1 changelog parenthetical from "(0/44)" to "(0/46)". |
| 2026-05-11 | Revision round 3 (reviewer feedback): corrected the `cafleet member create` recipe in *Agent Embedding Strategy* § *Codex-side dispatch* pattern 2 — the non-existent `--spawn-prompt-from-text` flag is replaced with the verified signature `cafleet --session-id <s> member create --agent-id <d> --name <n> --description <d> --coding-agent codex "<positional spawn prompt>"`; same fix in Step 5 codex check (c). Added two new Step 3 tasks to append the "codex inline-follow" and "codex member-spawn" subsections to each embedded spec — these two subsections, together with the existing Claude Code recipe subsection, satisfy the three-sibling-sub-heading contract in the Specification. Task count 0/47 → 0/49. |
| 2026-05-11 | Status promoted from `Draft` to `Approved` upon user approval relayed by the Director. Implementation steps verified actionable (no `[TBD]`, no standing `COMMENT(role): <body>` issue markers, every task carries the `<!-- completed: -->` timestamp slot). |
| 2026-05-11 | Implementation landed across 5 commits (b8e5f6c Step 1, 7e597c2 Step 2, 872dd6d Step 3, 79cd9f2 Step 4, 96bb4e9 Step 5) on branch `feat/promote-skills-agents-to-global`. PR #65 opened against `main`. Copilot review loop: 3 rounds — round 1 caught 3 issues (theme path documentation, embedded skill-name aliases for plugin installs, `Skill(base-dir)` misuse) fixed in 2db4fc7; round 2 caught 1 issue (`slide.md` vs `slides.md` naming inconsistency) fixed in d240fb9; round 3 returned "generated no new comments" — user treated as approval. Step 5 runtime smoke tests (out-of-repo Skill load, /create-figure render, codex compat (a)/(b)/(c)) are deferred to user-side post-merge sanity check; static-content verification recorded in Step 5 annotations. Status promoted to `Complete`. |
