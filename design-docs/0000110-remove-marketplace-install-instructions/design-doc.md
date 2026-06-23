# Remove marketplace install instructions

**Status**: Complete
**Progress**: 9/9 tasks complete
**Last Updated**: 2026-06-23

## Overview

Now that `cafleet setup` (design `0000109`) is the single end-user install path and `mise //:skill-install` is the contributor/local-dev path, the marketplace/plugin install instructions and their backing manifest files are redundant. This change removes every marketplace/plugin install mention from the docs and deletes the four marketplace manifest files (and the three corresponding `.bumpversion.toml` entries), leaving the repository reading as if marketplace-based installation never existed.

## Success Criteria

- [x] No marketplace/plugin install instruction remains in `docs/get-started/install.md`, `README.md`, or `docs/get-started/configure.md` (no `/plugin marketplace add`, `/plugin install`, `codex plugin marketplace add`, `gh skill install himkt/cafleet`, or `[marketplaces.cafleet]` config block).
- [x] The four manifest files are deleted: `.claude-plugin/marketplace.json`, `.claude-plugin/plugin.json`, `.codex-plugin/plugin.json`, `.agents/plugins/marketplace.json` (and the now-empty `.claude-plugin/`, `.codex-plugin/`, `.agents/` directories are gone).
- [x] The three `.bumpversion.toml` `[[tool.bumpversion.files]]` entries that targeted the deleted manifests are removed; a `bump-my-version` dry run succeeds against the remaining files.
- [x] `mise //:skill-install` (`gh skill install ./ --from-local`) still installs all three skills into all three agent homes **after** the manifests are deleted — empirically verified; this gates the manifest deletion.
- [x] `cafleet setup` (end-user) and `mise //:skill-install` (contributor) are the only two documented install paths; `cafleet setup` is the only end-user one.
- [x] A repo-wide search for the removed terms returns hits only inside `design-docs/` (the historical record).
- [x] `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:test` pass.

---

## Background

`cafleet setup` downloads the `cafleet-skills-v<version>.zip` Release asset (shipped by `.github/workflows/publish.yml`, which packages `skills/` only — never the manifests) and installs the skills into every detected coding-agent home, then migrates the database. The Release asset does **not** depend on the manifest files. Marketplace install therefore has no remaining end-user role.

Current marketplace/plugin install surface, from a full repo scan:

| Surface | Location | What it contains |
|---|---|---|
| Install doc | `docs/get-started/install.md` lines 64–80 | `gh skill install himkt/cafleet` (×3 agents), `/plugin marketplace add himkt/cafleet`, `/plugin install cafleet@cafleet`, `codex plugin marketplace add himkt/cafleet` |
| README | `README.md` line 28 | parenthetical `(or gh skill install / a coding-agent marketplace from the published repo)` |
| Configure doc | `docs/get-started/configure.md` lines 46–53 | Codex `[marketplaces.cafleet]` + `[plugins."cafleet@cafleet"]` config block; line 117 `"and the plugin enabled"` phrasing |
| Manifest | `.claude-plugin/marketplace.json` | Claude Code marketplace catalog |
| Manifest | `.claude-plugin/plugin.json` | Claude Code plugin manifest (lists the three skills) |
| Manifest | `.codex-plugin/plugin.json` | Codex plugin manifest (`"skills": "./skills/"`) |
| Manifest | `.agents/plugins/marketplace.json` | Codex marketplace catalog (URL source = `main`) |
| Version coupling | `.bumpversion.toml` lines 10–23 | three `[[tool.bumpversion.files]]` blocks bump `version` in `.codex-plugin/plugin.json`, `.claude-plugin/marketplace.json`, `.claude-plugin/plugin.json` |

**Out of scope / kept.** The contributor task `mise.toml` `[tasks.skill-install]` (and its use in `[tasks.all-install]`) is `gh skill install ./ --from-local` — the local working-tree path, not marketplace — and stays as the contributor/local-dev install. `.github/workflows/publish.yml` is unchanged (it never referenced the manifests). The agent-facing `skills/` directories and all other `docs/` pages are already clean.

**The one real risk — the `--from-local` coupling.** `gh skill install ./ --from-local` may read `.claude-plugin/plugin.json` / `.codex-plugin/plugin.json` to discover which skills to install. If it does, deleting those manifests would break the contributor task. This is settled empirically by a verification step (Step 3) that **gates** the manifest deletion: the deletion is only valid if `mise //:skill-install` still succeeds afterward.

---

## Specification

### Doc edits (Step 1 — documentation first)

#### `docs/get-started/install.md`

In the "Contributor / local-dev install" section, remove the published-repo / marketplace alternatives. Keep the `mise //:skill-install` block and its one-line description.

Remove this sentence and the two code blocks that follow it:

```text
You can also install from the published repository with the GitHub CLI or a
coding-agent marketplace:
```

```bash
gh skill install himkt/cafleet --agent claude-code
gh skill install himkt/cafleet --agent codex
gh skill install himkt/cafleet --agent opencode
```

```text
# Claude Code marketplace
/plugin marketplace add himkt/cafleet
/plugin install cafleet@cafleet

# Codex
codex plugin marketplace add himkt/cafleet
```

The sentence describing `mise //:skill-install` ends at "…into the three agent homes." and flows directly into the existing "Once the CLI and at least one coding-agent skill set are installed, continue to the Configure page…" closing paragraph.

#### `README.md`

Line 28 — drop the marketplace parenthetical, keep the contributor path:

| | Text |
|---|---|
| Before | `Contributors working from a clone install the skills from the working tree instead with `mise //:skill-install` (or `gh skill install` / a coding-agent marketplace from the published repo). Full install details…` |
| After | `Contributors working from a clone install the skills from the working tree with `mise //:skill-install`. Full install details…` |

#### `docs/get-started/configure.md`

1. Remove the Codex marketplace config block (lines 46–53): the `[marketplaces.cafleet]` table (`last_updated`, `last_revision`, `source_type`, `source`) **and** the `[plugins."cafleet@cafleet"]` `enabled = true` block. Keep the `[sandbox_workspace_write]` block — the Codex `config.toml` snippet then contains only `[sandbox_workspace_write]`.
2. Line 117 — remove the marketplace-residue phrasing "and the plugin enabled":

| | Text |
|---|---|
| Before | `Once the CLI is installed and the plugin enabled, you can build the documentation site (this site) locally with:` |
| After | `Once the CLI is installed, you can build the documentation site (this site) locally with:` |

### Manifest + version-coupling edits (Step 2)

| Action | Target |
|---|---|
| Delete file | `.claude-plugin/marketplace.json` |
| Delete file | `.claude-plugin/plugin.json` |
| Delete file | `.codex-plugin/plugin.json` |
| Delete file | `.agents/plugins/marketplace.json` |
| Remove now-empty dirs | `.claude-plugin/`, `.codex-plugin/`, `.agents/` (delete only if they hold nothing else) |
| Remove `.bumpversion.toml` blocks | the three `[[tool.bumpversion.files]]` entries for `.codex-plugin/plugin.json`, `.claude-plugin/marketplace.json`, `.claude-plugin/plugin.json` (lines 10–23). Keep the `cafleet/pyproject.toml` and `uv.lock` entries. |

After the `.bumpversion.toml` edit, the remaining version-bump targets are `cafleet/pyproject.toml` and `uv.lock` — the manifests no longer carry a `version` to keep in sync.

### Verification gate (Step 3)

The manifest deletion is only valid if the contributor install still works without the manifests. After Step 2, run `mise //:skill-install` and confirm `gh skill install ./ --from-local` succeeds for all three backends (`claude-code`, `codex`, `opencode`) and that the three skill directories land in `~/.claude/skills/`, `~/.codex/skills/`, and `~/.config/opencode/skills/`.

- **Pass** → the manifests were not required by `--from-local`; the removal stands.
- **Fail** → the deletion broke the contributor path. Do **not** silently restore a manifest or paper over it. Revert Step 2, record the exact `gh skill install` error, and escalate via `blocked (paragraph-Implementation > Step 3)` so the scope decision (keep a minimal manifest vs. change the contributor task) can be re-made with the user. The empirical result decides; the design does not guess.

This is a real-command verification run in the working tree, consistent with the project's authorization-scope rules — it is delegated to whoever runs the implementation, not faked.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation first

- [x] Edit `docs/get-started/install.md`: remove the "You can also install from the published repository…" sentence and the two following code blocks (`gh skill install himkt/cafleet` ×3 and the `/plugin marketplace add` / `/plugin install` / `codex plugin marketplace add` block); keep the `mise //:skill-install` block flowing into the Configure-page closing paragraph. <!-- completed: 2026-06-23T12:21 -->
- [x] Edit `README.md` line 28: drop the `(or gh skill install / a coding-agent marketplace from the published repo)` parenthetical, keeping the `mise //:skill-install` contributor sentence. <!-- completed: 2026-06-23T12:21 -->
- [x] Edit `docs/get-started/configure.md`: remove the `[marketplaces.cafleet]` + `[plugins."cafleet@cafleet"]` blocks (keep `[sandbox_workspace_write]`), and change "Once the CLI is installed and the plugin enabled" to "Once the CLI is installed". <!-- completed: 2026-06-23T12:21 -->

### Step 2: Delete manifests and version coupling

- [x] Delete `.claude-plugin/marketplace.json`, `.claude-plugin/plugin.json`, `.codex-plugin/plugin.json`, `.agents/plugins/marketplace.json`; remove the `.claude-plugin/`, `.codex-plugin/`, `.agents/` directories if they are then empty. <!-- completed: 2026-06-23T12:58 -->
- [x] Remove the three `[[tool.bumpversion.files]]` blocks in `.bumpversion.toml` that target the deleted manifests (keep the `cafleet/pyproject.toml` and `uv.lock` blocks). <!-- completed: 2026-06-23T12:24 -->

### Step 3: Verification gate (gates Step 2)

- [x] Run `mise //:skill-install` after the manifests are deleted and confirm `gh skill install ./ --from-local` succeeds for all three backends, with the three skill directories present under `~/.claude/skills/`, `~/.codex/skills/`, and `~/.config/opencode/skills/`. On failure, revert Step 2 and escalate `blocked (paragraph-Implementation > Step 3)` with the exact error — do not restore a partial manifest silently. <!-- completed: 2026-06-23T13:08 -->
- [x] Run `bump-my-version` in dry-run mode (e.g. `uv run bump-my-version bump --dry-run patch`) and confirm it resolves cleanly against the remaining files (no reference to the deleted manifests). <!-- completed: 2026-06-23T13:08 -->

### Step 4: Sweep and validate

- [x] Repo-wide `git grep` (tracked files only — excludes `node_modules/`, the gitignored `site/` build artifacts, and `researches/`) for `marketplace`, `/plugin install`, `/plugin marketplace`, `codex plugin marketplace`, `gh skill install himkt/cafleet`, `.claude-plugin`, `.codex-plugin`, `.agents/plugins` — confirm the only remaining hits are inside `design-docs/` (the historical record, including this doc). The term `gh skill install himkt/cafleet` is the specific published-repo form and will not match `mise.toml`'s kept `gh skill install ./ --from-local`. Fix any stray hit found outside `design-docs/`. <!-- completed: 2026-06-23T13:10 -->
- [x] Run `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:test`; fix any findings. <!-- completed: 2026-06-23T13:10 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-23 | Initial draft |
| 2026-06-23 | Implemented via CAFleet execute team. Steps 1–4 complete: marketplace/plugin install instructions removed from README.md, install.md, configure.md; four manifest files and three .bumpversion.toml blocks deleted. Verification gate passed (skill-install works without manifests; bump-my-version dry run clean). Sweep clean (removed terms only in design-docs/); lint, typecheck, 928 tests pass. Independent Opus review and Copilot review both clean. Status → Complete. |
