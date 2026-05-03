# Codex Plugin Manifest

**Status**: Approved
**Progress**: 0/7 tasks complete
**Last Updated**: 2026-05-03

## Overview

Add a Codex plugin manifest at `.codex-plugin/plugin.json` plus a Codex marketplace at `.agents/plugins/marketplace.json` so the existing CAFleet skills can be installed via `codex plugin marketplace add himkt/cafleet` followed by an in-UI install. The manifest reuses the current `skills/` tree as-is and coexists with the existing Claude Code plugin under `.claude-plugin/`.

## Success Criteria

- [ ] `.codex-plugin/plugin.json` exists at the repo root with the four required fields (`name`, `version`, `description`, `skills`).
- [ ] `.agents/plugins/marketplace.json` exists and references the in-repo plugin so the repo itself is a Codex marketplace.
- [ ] The Claude Code plugin under `.claude-plugin/` continues to function unchanged (same 7 skills, same `marketplace.json`, same versions).
- [ ] `README.md` documents the Codex install path alongside the existing Claude Code install path.
- [ ] `ARCHITECTURE.md` notes that CAFleet ships dual plugin manifests (Claude + Codex) over a shared `skills/` tree.
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
  "description": "A2A-inspired message broker CLI and design document orchestration skills for coding agents.",
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
        "source": "local",
        "path": "./"
      },
      "policy": {
        "installation": "AVAILABLE"
      }
    }
  ]
}
```

Field rules:

| Field | Value | Notes |
|---|---|---|
| `name` (top-level) | `"cafleet"` | Marketplace name. Mirrors `.claude-plugin/marketplace.json:name` for symmetry. |
| `plugins[0].name` | `"cafleet"` | Must match `plugin.json:name`. |
| `plugins[0].source` | object with `source: "local"` and `path: "./"` | The plugin manifest at `.codex-plugin/plugin.json` lives at the repo root. Codex's resolver looks for `<marketplace-parent>/<path>/.codex-plugin/plugin.json`; from `.agents/plugins/marketplace.json` the marketplace parent is `.agents/plugins/`, so the path that points back at the repo root is the operator-verified part of the design (see Implementation Step 4). The spec'd value is `"./"`; the documented fallback is `"../../"`. |
| `plugins[0].policy.installation` | `"AVAILABLE"` | Plugin appears in the in-UI install list. No `authentication` field; the plugin needs no credentials. |

Out of scope for v1 (intentionally omitted): `interface` (display metadata), `category`, `authentication: ON_INSTALL` (no creds). The minimal three fields above are enough for `codex plugin marketplace add himkt/cafleet` to succeed.

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

- [ ] Update `README.md`: add a "Install the plugin in Codex" subsection alongside the existing "Install the plugin in Claude Code" block (§Install). Document the `codex plugin marketplace add himkt/cafleet` command and the in-UI install step. Note that the same 7 skills land in Codex. <!-- completed: -->
- [ ] Update `ARCHITECTURE.md`: add a sentence (in an appropriate "Distribution" / "Plugin packaging" section, or near the top if no such section exists) noting that CAFleet ships dual plugin manifests (Claude Code at `.claude-plugin/`, Codex at `.codex-plugin/` + `.agents/plugins/marketplace.json`) over a shared `skills/` tree. <!-- completed: -->

### Step 2: Add the Codex plugin manifest

- [ ] Create `.codex-plugin/plugin.json` with the exact JSON specified in §Specification → File 1. <!-- completed: -->

### Step 3: Add the Codex marketplace catalog

- [ ] Create `.agents/plugins/marketplace.json` with the exact JSON specified in §Specification → File 2. <!-- completed: -->

### Step 4: Manual install verification

`codex plugin marketplace add himkt/cafleet` fetches the marketplace from GitHub, so the implementation branch MUST be reachable on the public repo before this step runs. Either land the changes on the default branch (`main`), or push the branch and pass the appropriate ref to the marketplace-add command per Codex's documented syntax. Local-only changes will not satisfy this step.

- [ ] Push the implementation branch to GitHub so `codex plugin marketplace add himkt/cafleet` can fetch it. The simplest path is to merge to `main`; alternatively, push the branch and pass its ref to the marketplace-add command. Local commits alone do NOT satisfy this task. <!-- completed: -->
- [ ] Operator runs `codex plugin marketplace add himkt/cafleet` against the now-public branch and completes the in-UI install. Confirms that all 7 skills (`cafleet`, `agent-team-monitoring`, `agent-team-supervision`, `design-doc`, `design-doc-create`, `design-doc-execute`, `design-doc-interview`) are exposed. <!-- completed: -->
- [ ] If the install fails because Codex cannot resolve `plugins[0].source.path`, patch `.agents/plugins/marketplace.json`. The spec'd value is `"./"`; the documented fallback is `"../../"` (since the marketplace parent is `.agents/plugins/`, two levels deep from the repo root). Try the fallback, re-run the marketplace-add + UI install, and update §Specification → File 2 to match the value that works. This is the only known implementation risk; the operator's first smoke test resolves it. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-03 | Initial draft |
| 2026-05-03 | Reviewer round 1: fix progress count (now 0/7), correct Step 4 fallback path math (`../../`, not `../`), make GitHub-push prerequisite explicit, fold version/description/skill invariants into a Constraints & invariants subsection, list the 7 skills in Success Criteria. |
| 2026-05-03 | User approved. Status → Approved. |
