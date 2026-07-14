# Codex Docs Cleanup

**Status**: Complete
**Progress**: 11/11 tasks complete
**Last Updated**: 2026-07-14

## Overview

Docs-only follow-up to GitHub issue #196 ("Codex permission problems"): correct the Codex configuration snippets (sandbox TOML and `cafleet.rules`), consolidate the Install / Configure / Quickstart pages into a single simplified Quickstart, and make every top-level docs-site nav section collapse by default. No code changes.

## Success Criteria

- [x] Both Codex sandbox TOML snippets (merged Quickstart and `docs/reference/coding-agents/codex.md`) contain `network_access = true` plus the `writable_roots` entry, each with the tmux/herdr socket rationale.
- [x] The `cafleet.rules` snippet is the two-rule form from issue #196, the prose states that the more specific prefix rule takes precedence, and a `!!! tip "Where this lives"` admonition names `~/.codex/rules/cafleet.rules`.
- [x] `docs/get-started/` contains exactly `index.md`, `quickstart.md`, and `contributing.md`; a repo-wide search for links to `install.md` / `configure.md` (and their published URLs) returns nothing.
- [x] The contributor skill install (`mise //:skill-install`) is documented in `docs/get-started/contributing.md`.
- [x] `README.md`'s "Full guide" link points to the Quickstart page and the Get Started blurb no longer names Install/Configure.
- [x] `navigation.sections` is absent from `zensical.toml`; `mise //:docs-build` succeeds and the served site shows all top-level nav sections collapsed by default (matching how "Coding-agent backends" behaves today).

---

## Background

Issue #196 reports two Codex problems the current docs do not solve:

1. Codex members fail with `Error: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }` because the tmux/herdr multiplexer backends communicate over a socket, which Codex's `workspace-write` sandbox classifies as network access. The documented `[sandbox_workspace_write]` snippet lacks `network_access = true`.
2. The documented 14-rule `cafleet.rules` enumeration is unnecessary. The issue comment empirically confirms that a broad `["cafleet"]` allow plus a specific `["cafleet", "member", "exec"]` prompt rule is sufficient — the more specific prefix rule takes precedence. The current `configure.md` prose asserts the opposite ("a broad `["cafleet"]` allow would also cover `cafleet member exec`"), which is incorrect.

Separately, the get-started flow is spread over three wordy pages (Install → Configure → Quickstart), and the "How-to guides" nav section always renders expanded because the `navigation.sections` theme feature keeps top-level sections open (only nested groups such as "Coding-agent backends" collapse).

---

## Specification

### 1. Corrected Codex sandbox TOML (two occurrences)

The `[sandbox_workspace_write]` snippet appears in two places — the merged Quickstart's Codex subsection and the `> [!IMPORTANT]` callout in `docs/reference/coding-agents/codex.md` (currently lines 35–43). Both become:

```toml
[sandbox_workspace_write]
network_access = true
writable_roots = ["/home/<you>/.local/share/cafleet"]
```

Each occurrence is accompanied by this rationale (adapted to the surrounding prose):

- `network_access = true` is required because cafleet's multiplexer backends (tmux and herdr) communicate over a local socket, which the Codex sandbox classifies as network access; without it cafleet commands fail with `Operation not permitted`.
- `writable_roots` grants write access to cafleet's default SQLite DB directory. Keep the placeholder path form `/home/<you>/.local/share/cafleet`. Both occurrences carry the sentence "Use the absolute path matching `CAFLEET_DATABASE_URL` or the default XDG location." — kept as-is in `codex.md` (where it exists today at line 43) and added to the merged Quickstart's Codex subsection (its `configure.md` source has no such sentence).

### 2. Corrected `cafleet.rules` (two-rule form)

The 14-rule enumeration in the Codex configure content is replaced by:

```text
prefix_rule(pattern = ["cafleet"], decision = "allow")

prefix_rule(
    pattern = ["cafleet", "member", "exec"],
    decision = "prompt",
    justification = "cafleet member exec runs arbitrary commands on a member",
)
```

The merged page's `### Codex` subsection therefore carries two "Where this lives" admonitions: the existing one naming `~/.codex/config.toml` (kept from `configure.md:41-43`, introducing the §1 sandbox TOML) and a new one for the rules file. The rules snippet is introduced by that new admonition — the "Where this lives" section required by the request:

```markdown
!!! tip "Where this lives"

    The Codex rules for `cafleet` commands live at `~/.codex/rules/cafleet.rules`.
```

The two-paragraph rationale prose (current `configure.md:75-87`) is replaced by:

> The more specific prefix rule takes precedence: `["cafleet", "member", "exec"]` wins over the broad `["cafleet"]` allow, so `cafleet member exec` keeps prompting while every other subcommand is allowed. Because `--fleet-id` is a trailing per-subcommand flag, it sits past the matched prefix — no per-fleet rule is needed.

### 3. Merged Quickstart page

`docs/get-started/quickstart.md` absorbs `install.md` and `configure.md`; those two files are deleted. Section outline of the merged page:

| Merged section | Source | Content after the merge |
|---|---|---|
| intro | quickstart.md | 1–2 sentences; drop the "assumes you have already followed Install and Configure" sentence |
| `## Install` | install.md | Prerequisites (3 condensed bullets); the `uv tool install cafleet` + `cafleet setup` block; one line each for `cafleet setup db` and `cafleet setup skill [--agent ...]`; one stale-skills sentence quoting the error string `` `stale skills detected (...); run 'cafleet setup skill' to reinstall` ``; one sentence on the default DB path `~/.local/share/cafleet/cafleet_v5.db` + `CAFLEET_DATABASE_URL` override |
| `## Configure` | configure.md | Intro sentence; `### Claude Code` (existing "Where this lives" admonition + JSON block + glob prose, unchanged); `### Codex` (the kept `~/.codex/config.toml` "Where this lives" admonition, the §1 TOML + rationale, then the §2 `cafleet.rules` admonition + snippet + precedence prose); `### Opencode` (existing content, unchanged); `### Trust the working directory` (unchanged); `### Passing the fleet id` (unchanged) |
| `## Simple example — invoke from a coding agent` | quickstart.md | Kept unchanged |
| `## Design-doc-driven development` | quickstart.md | Kept unchanged |
| `## Raw CLI walkthrough` | quickstart.md | Kept unchanged (collapsible `??? example` block) |
| Where to go next | quickstart.md | Kept unchanged |

Content CUT (deleted, not relocated), per the confirmed simplification list:

- The "Stale-skills detection" deep-dive (only the single sentence + error string above survives).
- The `!!! note "Upgrading an existing database"` admonition.
- The long prose explaining `cafleet setup`'s two halves (the two subcommand lines above survive).

Content RELOCATED:

- "Contributor / local-dev install" (`mise //:skill-install` and its `gh skill install` explanation) moves to `docs/get-started/contributing.md`, as a short subsection under `## Development`.

The demoted Configure headings keep their heading text, so the Zensical toc slugs `#configure`, `#claude-code`, `#codex`, `#opencode`, `#trust-the-working-directory`, and `#passing-the-fleet-id` are generated on the merged page (no duplicate headings exist there); `#install` comes from the new `## Install` heading.

### 4. Navigation and link updates

`zensical.toml` changes:

| Change | Detail |
|---|---|
| Get Started nav | `[ "get-started/index.md", { "Quickstart" = "get-started/quickstart.md" }, { "Contributing" = "get-started/contributing.md" } ]` — the Install and Configure rows are removed |
| Theme features | Remove `"navigation.sections"` from `[project.theme].features`. All top-level sections (Get Started, How-to guides, Concepts, Specification, API Reference) then render as collapsible groups, collapsed by default unless they contain the active page — the same behavior "Coding-agent backends" has today. The site-wide look change is accepted and intended. `navigation.indexes` and the other features stay. |

Inbound link retargets (no redirect stubs; the old published URLs 404):

| File | Current link | New link |
|---|---|---|
| `docs/get-started/index.md` | bullets for Install / Configure / Quickstart | Rewrite as two bullets: Quickstart (described as "install, configure, and run your first fleet") and Contributing; drop the "Start with Install…" intro sentence in favor of one pointing at Quickstart |
| `docs/spec/cli-options.md:189` | `[Install](../get-started/install.md)` | `[Quickstart](../get-started/quickstart.md#install)` |
| `docs/how-to/mixed-backend-team.md:17` | `[Install](../get-started/install.md)` | `[Quickstart § Install](../get-started/quickstart.md#install)` |
| `docs/how-to/mixed-backend-team.md:18` | `[Configure](../get-started/configure.md)` | `[Quickstart § Configure](../get-started/quickstart.md#configure)` |
| `docs/reference/coding-agents/codex.md:45` | `[Configure § Trust the working directory](../../get-started/configure.md#trust-the-working-directory)` | `[Quickstart § Trust the working directory](../../get-started/quickstart.md#trust-the-working-directory)` |
| `README.md:14` | `<https://himkt.github.io/cafleet/get-started/install/>` | `<https://himkt.github.io/cafleet/get-started/quickstart/>` |
| `README.md:18` | "install, configure, quickstart, contributing" | "quickstart and contributing" |
| `.claude/skills/update-readme/SKILL.md:34` | "align … the Install block with docs/get-started/install.md" | "align … the Install block with the Install section of docs/get-started/quickstart.md" |

`docs/concepts/overview.md:91` already links `quickstart.md` and needs no edit. `SPEC.md` and `skills/` carry none of the Codex config snippets and do not describe the docs-site nav, so they are unaffected; under `.claude/skills/`, the only affected file is `update-readme/SKILL.md`, retargeted above.

### 5. Verification

`mise //:docs-build` (the existing Zensical wrapper documented in `contributing.md`) is the build gate; do NOT add a new mise docs task. The nav-collapse behavior and the merged page are verified manually: run `uv run zensical serve` (Zensical's preview server, default `localhost:8000`), confirm every top-level section renders collapsed by default, open the merged Quickstart, and click through each retargeted link. The operator runs the serve command by hand — it stays out of mise per the decision above.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Codex snippet correction in the reference page

- [x] Update the `> [!IMPORTANT]` callout in `docs/reference/coding-agents/codex.md`: add `network_access = true` to the TOML and the socket-rationale sentence (§1) <!-- completed: 2026-07-14T20:39 -->

### Step 2: Consolidate the get-started pages

- [x] Rewrite `docs/get-started/quickstart.md` as the merged simplified page per §3, with the corrected Codex snippets (§1, §2) and the `cafleet.rules` "Where this lives" admonition <!-- completed: 2026-07-14T20:43 -->
- [x] Add the contributor skill-install content to `docs/get-started/contributing.md` under `## Development` <!-- completed: 2026-07-14T20:43 -->
- [x] Delete `docs/get-started/install.md` and `docs/get-started/configure.md` <!-- completed: 2026-07-14T20:44 -->
- [x] Update `docs/get-started/index.md` per the §4 retarget table <!-- completed: 2026-07-14T20:43 -->

### Step 3: Navigation, links, and README

- [x] Update `zensical.toml`: Get Started nav rows and removal of `navigation.sections` (§4) <!-- completed: 2026-07-14T20:47 -->
- [x] Retarget the inbound links in `docs/spec/cli-options.md`, `docs/how-to/mixed-backend-team.md`, and `docs/reference/coding-agents/codex.md` (§4) <!-- completed: 2026-07-14T20:47 -->
- [x] Update `README.md`: the "Full guide" link and the Get Started blurb (§4) <!-- completed: 2026-07-14T20:47 -->
- [x] Retarget `.claude/skills/update-readme/SKILL.md` to the merged Quickstart's Install section (§4) <!-- completed: 2026-07-14T20:53 -->

### Step 4: Verification

- [x] Run `mise //:docs-build`; serve the site and verify: all top-level nav sections collapsed by default, the merged Quickstart renders with the corrected snippets, and every retargeted link resolves (§5) <!-- completed: 2026-07-14T21:04 -->
- [x] Repo-wide search confirms no remaining reference to `install.md` / `configure.md` or their published URLs <!-- completed: 2026-07-14T21:04 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-14 | Initial draft |
| 2026-07-14 | Review round 1: dual "Where this lives" admonitions in the Codex subsection, `CAFLEET_DATABASE_URL` sentence in both TOML occurrences, `update-readme` skill retarget row + task, named `uv run zensical serve` as the verification serve command |
| 2026-07-14 | Implementation complete: all 11 tasks and 6 Success Criteria verified (Verifier E2E + Reviewer approved round 1); PR #197 opened; Status → Complete |
