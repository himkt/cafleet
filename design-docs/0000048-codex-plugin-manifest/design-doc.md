# Codex Plugin Manifest

**Status**: Complete
**Progress**: 6/7 tasks complete
**Last Updated**: 2026-05-05

## Overview

Add a Codex plugin manifest at `.codex-plugin/plugin.json` plus a Codex marketplace at `.agents/plugins/marketplace.json` so the existing CAFleet skills can be installed via `codex plugin marketplace add himkt/cafleet` followed by an in-UI install. The manifest reuses the current `skills/` tree as-is and coexists with the existing Claude Code plugin under `.claude-plugin/`.

## Success Criteria

- [x] `.codex-plugin/plugin.json` exists at the repo root with the four required fields (`name`, `version`, `description`, `skills`).
- [x] `.agents/plugins/marketplace.json` exists and references the in-repo plugin so the repo itself is a Codex marketplace.
- [x] The Claude Code plugin under `.claude-plugin/` continues to function unchanged (same 7 skills, same `marketplace.json`, same versions).
- [x] `README.md` documents the Codex install path alongside the existing Claude Code install path.
- [x] `ARCHITECTURE.md` notes that CAFleet ships dual plugin manifests (Claude + Codex) over a shared `skills/` tree.
- [ ] Manual `codex plugin marketplace add himkt/cafleet` followed by an in-UI install succeeds and exposes all 7 skills: `cafleet`, `agent-team-monitoring`, `agent-team-supervision`, `design-doc`, `design-doc-create`, `design-doc-execute`, `design-doc-interview` (operator-verified at execute time).

---

## Background

CAFleet currently ships as a Claude Code plugin: `.claude-plugin/plugin.json` declares the 7 skills listed under Success Criteria above (under `skills/`), and `.claude-plugin/marketplace.json` makes the repo itself an installable marketplace. Codex (the OpenAI CLI) has a parallel plugin system documented at <https://developers.openai.com/codex/plugins/build>: the manifest lives at `.codex-plugin/plugin.json` and the marketplace catalog at `.agents/plugins/marketplace.json`.

Both systems read `SKILL.md` files in the same shape, so the existing skills work without any rewrite — only the manifest and marketplace files are new.

---

## Specification

### File 1: `.codex-plugin/plugin.json`

Minimal manifest. Codex's `skills` field accepts a directory string (`"./skills/"`) and auto-bundles every skill it finds, so no per-skill array is needed.

```json
{
  "name": "cafleet",
  "version": "0.3.0",
  "description": "Message broker CLI and design document orchestration skills for coding agents.",
  "skills": "./skills/"
}
```

Field rules:

| Field | Value | Notes |
|---|---|---|
| `name` | `"cafleet"` | Mirrors `.claude-plugin/plugin.json:name`. |
| `version` | `"0.3.0"` | Locked in lock-step with `.claude-plugin/plugin.json:version` (see Constraints). |
| `description` | identical to Claude side | Single source of truth: copy from `.claude-plugin/plugin.json:description` verbatim (see Constraints). |
| `skills` | `"./skills/"` | Directory string, NOT an array. Codex auto-discovers every `SKILL.md` under the directory; this exposes all 7 existing skills with no extra bookkeeping. |

Out of scope for v1 (intentionally omitted): `author`, `homepage`, `repository`, `license`, `keywords`, `mcpServers`, `apps`, `hooks`, `interface`. Add later if Codex install or discovery surfaces require them; they are not needed for `codex plugin marketplace add` to work.

### File 2: `.agents/plugins/marketplace.json`

Codex marketplace catalog so the repo root is itself a marketplace addressable via `codex plugin marketplace add himkt/cafleet`.

```json
{
  "name": "cafleet",
  "plugins": [
    {
      "name": "cafleet",
      "source": {
        "source": "url",
        "url": "https://github.com/himkt/cafleet.git",
        "ref": "main"
      },
      "policy": {
        "installation": "AVAILABLE",
        "authentication": "ON_INSTALL"
      },
      "category": "Productivity"
    }
  ]
}
```

Field rules:

| Field | Value | Notes |
|---|---|---|
| `name` (top-level) | `"cafleet"` | Marketplace name. Mirrors `.claude-plugin/marketplace.json:name` for symmetry. |
| `plugins[0].name` | `"cafleet"` | Must match `plugin.json:name`. |
| `plugins[0].source.source` | `"url"` | Literal string. The Codex docs at <https://developers.openai.com/codex/plugins/build> prescribe `"source": "url"` for plugins that live at the repository root and `"source": "git-subdir"` for plugins in a subdirectory. The earlier `"local"` value (round 2) was wrong because local-source paths must stay inside the marketplace root (`.agents/plugins/`), and our plugin lives at the repo root. |
| `plugins[0].source.url` | `"https://github.com/himkt/cafleet.git"` | The HTTPS git URL of this repository. Codex clones from this URL when resolving the plugin. |
| `plugins[0].source.ref` | `"main"` | The git ref Codex checks out. Refreshable — bump alongside any future `version` change so installs pull the matching tagged or branch tip. |
| `plugins[0].policy.installation` | `"AVAILABLE"` | Plugin appears in the in-UI install list. |
| `plugins[0].policy.authentication` | `"ON_INSTALL"` | Required by the spec on every plugin entry. CAFleet needs no credentials, so this is a no-op for the install flow; `"ON_INSTALL"` is the only documented value, included for spec compliance. |
| `plugins[0].category` | `"Productivity"` | Top-level on the plugin entry (NOT under `policy`). Required by the spec on every plugin entry; `"Productivity"` is the only documented value. |

Out of scope for v1 (intentionally omitted): `interface` (display metadata). The fields above are enough for `codex plugin marketplace add himkt/cafleet` to succeed.

### Coexistence with the Claude Code plugin

| Path | Owner | Status |
|---|---|---|
| `.claude-plugin/plugin.json` | Claude Code | Unchanged. Continues to list each of the 7 skills explicitly (per its schema). |
| `.claude-plugin/marketplace.json` | Claude Code | Unchanged. |
| `.codex-plugin/plugin.json` | Codex | NEW. Single `"skills": "./skills/"` directory string covers all 7 skills. |
| `.agents/plugins/marketplace.json` | Codex | NEW. Repo-root marketplace. |
| `skills/*/SKILL.md` | shared | Unchanged. Same files serve both plugins. |

The two manifests use different `skills` schemas (Claude: explicit array of paths; Codex: single directory string), but both anchor at the same `skills/` tree. No file moves, no duplication, no shim layer.

### Constraints & invariants

The following are true by construction, not work items. They MUST hold at every commit on this branch and in all future bumps.

- **Lock-step `version`**: `.claude-plugin/plugin.json:version` and `.codex-plugin/plugin.json:version` are byte-identical. Both files move together on every future bump (they are version-pinned to one another, not to a third source).
- **Lock-step `description`**: `.claude-plugin/plugin.json:description` and `.codex-plugin/plugin.json:description` are byte-identical. Same single-edit rule on future updates.
- **No skill-content edits**: `skills/*/SKILL.md` is not modified by this design. The skills are unchanged; only manifest plumbing is added.
- **Claude plugin untouched**: `.claude-plugin/plugin.json` and `.claude-plugin/marketplace.json` are not modified by this design.

A reviewer or executor who finds either lock-step invariant broken on this branch MUST treat it as a blocker.

### Out of scope

The following are explicitly NOT part of this design and will not be implemented in this cycle:

- Publishing CAFleet to a public Codex plugin directory.
- MCP servers (`mcpServers` field) — none today, none added here.
- Custom apps (`apps` field) — none today, none added here.
- Marketplace UI polish (`interface` block, icons, displayName, descriptions, links, default prompts).
- Skill rewrites — `skills/*/SKILL.md` content is not touched.
- Codex CI lint or automated schema validation for the new JSON files.
- A Codex-specific test harness.

### Verification approach

Manual operator verification is the only acceptance check. There is no JSON-schema lint, no CI hook, and no automated install test. The full procedure (push branch → run command → UI install → confirm 7 skills → patch path on failure) is captured in Implementation Step 4.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation first

- [x] Update `README.md`: add a "Install the plugin in Codex" subsection alongside the existing "Install the plugin in Claude Code" block (§Install). Document the `codex plugin marketplace add himkt/cafleet` command and the in-UI install step. Note that the same 7 skills land in Codex. <!-- completed: 2026-05-05T13:42 -->
- [x] Update `ARCHITECTURE.md`: add a sentence (in an appropriate "Distribution" / "Plugin packaging" section, or near the top if no such section exists) noting that CAFleet ships dual plugin manifests (Claude Code at `.claude-plugin/`, Codex at `.codex-plugin/` + `.agents/plugins/marketplace.json`) over a shared `skills/` tree. <!-- completed: 2026-05-05T13:42 -->

### Step 2: Add the Codex plugin manifest

- [x] Create `.codex-plugin/plugin.json` with the exact JSON specified in §Specification → File 1. <!-- completed: 2026-05-05T13:48 -->

### Step 3: Add the Codex marketplace catalog

- [x] Create `.agents/plugins/marketplace.json` with the exact JSON specified in §Specification → File 2. <!-- completed: 2026-05-05T13:50 -->

### Step 4: Manual install verification

`codex plugin marketplace add himkt/cafleet` fetches the marketplace from GitHub, so the implementation branch MUST be reachable on the public repo before this step runs. Either land the changes on the default branch (`main`), or push the branch and pass the appropriate ref to the marketplace-add command per Codex's documented syntax. Local-only changes will not satisfy this step.

- [x] Push the implementation branch to GitHub so `codex plugin marketplace add himkt/cafleet` can fetch it. The simplest path is to merge to `main`; alternatively, push the branch and pass its ref to the marketplace-add command. Local commits alone do NOT satisfy this task. <!-- completed: 2026-05-05T13:51 -->
- [ ] Operator runs `codex plugin marketplace add himkt/cafleet` against the now-public branch and completes the in-UI install. Confirms that all 7 skills (`cafleet`, `agent-team-monitoring`, `agent-team-supervision`, `design-doc`, `design-doc-create`, `design-doc-execute`, `design-doc-interview`) are exposed. <!-- completed: -->
- [x] If the install fails because Codex cannot resolve the plugin manifest, patch `.agents/plugins/marketplace.json` so the `source` shape matches what the Codex docs at <https://developers.openai.com/codex/plugins/build> prescribe for repo-root plugins. The doc's verbatim guidance is to use `"source": "url"` when the plugin lives at the repository root and `"source": "git-subdir"` when it lives in a subdirectory. The catalog also requires `policy.authentication` and a top-level `category` on every plugin entry. Update `.agents/plugins/marketplace.json` and §Specification → File 2 (spec block + field-rules table + out-of-scope subsection) together. <!-- completed: 2026-05-05T13:59 (applied in Executor round 3 in response to user-driven docs verification: source-type pivot from local to url since plugin lives at repo root, plus added policy.authentication and category required by the spec) -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-03 | Initial draft |
| 2026-05-03 | Reviewer round 1: fix progress count (now 0/7), correct Step 4 fallback path math (`../../`, not `../`), make GitHub-push prerequisite explicit, fold version/description/skill invariants into a Constraints & invariants subsection, list the 7 skills in Success Criteria. |
| 2026-05-03 | User approved. Status → Approved. |
| 2026-05-05 | Executor round 1: spec drift fix — File 1 description string aligned with current `.claude-plugin/plugin.json:description` (dropped `"A2A-inspired "` prefix) per lock-step invariant + Claude-untouched constraint. |
| 2026-05-05 | Executor round 2: applied path-fallback `"../../"` preemptively in response to Copilot review (PR #55), per design doc Step 4.3. The resolver formula stated in §Specification → File 2 (`<marketplace-parent>/<path>/.codex-plugin/plugin.json`) makes `"./"` mathematically unable to reach the repo-root manifest from `.agents/plugins/`; `"../../"` resolves correctly. Both `.agents/plugins/marketplace.json` and §Specification → File 2 (spec block + field-rules table) updated together. |
| 2026-05-05 | Executor round 3: source-type pivot. Direct read of <https://developers.openai.com/codex/plugins/build> showed that the round-2 path-math fallback (`"../../"`) was unfixable: local-source paths must stay inside the marketplace root, but our plugin lives at the repo root. Switched to `source.source = "url"` with `url = https://github.com/himkt/cafleet.git`, `ref = "main"` — the doc-prescribed source type for repo-root plugins. Also added `policy.authentication = "ON_INSTALL"` and `category = "Productivity"` (both required per the doc's "Always include" rule). §Specification → File 2 spec block + field-rules table + out-of-scope subsection updated together; `.agents/plugins/marketplace.json` rewritten to match. |
| 2026-05-05 | Executor round 4: skill-tree boundary fix. Copilot's round-3 review (PR #55) flagged that `skills/update-readme/SKILL.md` was an 8th sibling under `skills/`, so the Codex manifest's `"skills": "./skills/"` directory string would auto-discover 8 skills instead of the 7 the design + README + ARCHITECTURE document. The Claude-side `.claude-plugin/plugin.json:skills` array already excluded `update-readme` (it was always project-local). Resolution: `git mv skills/update-readme .claude/skills/update-readme` — Claude Code's project-skill discovery still picks it up via `.claude/skills/`, neither plugin manifest now sees it, and the documented 7-skill count becomes accurate for both plugins. No content edits to the SKILL.md file (the `Constraints & invariants` rule "skills/*/SKILL.md content is not touched" is preserved — only the file's parent path moved). |
| 2026-05-05 | Status → Complete. After the round-4 push, Copilot's re-review on commit 6090e40 returned `state: COMMENTED, body: "reviewed 5 out of 6 changed files in this pull request and generated no new comments"` with 0 inline comments — its all-clear pattern (Copilot rarely emits a formal `APPROVED` state). User accepted this as the loop exit signal. SC6 / Implementation Step 4.2 (operator-verified `codex plugin marketplace add himkt/cafleet` install + 7 skills exposed) remains unchecked: it is the only task that requires operator action against a public ref, and the user opted to defer it past the loop exit rather than run a pre-merge smoke test. The design's spec-alignment with <https://developers.openai.com/codex/plugins/build> (verified across rounds 3–4: `source.source = "url"`, `policy.authentication`, `category`, exactly 7 auto-discoverable skills under `./skills/`) is the basis for shipping without the empirical install round-trip. |
