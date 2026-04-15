# Cleanup and Refactor — Post-Rapid-Iteration Debt Payment

**Status**: Complete
**Progress**: 50/50 tasks complete
**Last Updated**: 2026-04-15

## Overview

Pay down accumulated tech debt from the recent rapid iteration (0000011 remove-mcp-server → 0000023 session-id-cli-flag-docs): delete the dead `vendor/A2A` submodule and stale `docs/spec/` files, fix a real `_resolve_prompt` custom-prompt bug, and unify `session` vs `namespace` wording across README / ARCHITECTURE / SKILL.md / docs. Ship in three independently-mergeable phases: low-risk deletions, then the code fix, then naming unification.

## Success Criteria

- [x] `vendor/A2A` submodule is fully removed from the working tree, `.gitmodules`, and every rule / skill / CLAUDE.md reference.
- [x] `docs/spec/a2a-operations.md` is deleted; `docs/spec/registry-api.md` is deleted or collapsed into `docs/spec/webui-api.md`; `docs/spec/data-model.md` no longer claims an `a2a-sdk` / `aiosqlite` dependency that does not exist.
- [x] `cafleet/src/cafleet/cli.py:_resolve_prompt` substitutes `{session_id}` / `{agent_id}` / `{director_name}` / `{director_agent_id}` on BOTH the default prompt template AND user-supplied `prompt_argv`. A regression test in `cafleet/tests/test_cli_member.py` (new file) pins the behaviour.
- [x] `grep -rn 'namespace' README.md ARCHITECTURE.md CLAUDE.md .claude/CLAUDE.md docs/spec/cli-options.md` returns zero occurrences where "namespace" is used as a synonym for "session", **except** the single explicit dual-name disambiguation form `session (namespace)` left at `docs/spec/cli-options.md:25` only.
- [x] `grep -rn 'Tenant isolation' .` (excluding `design-docs/`, `vendor/`) returns zero matches. (Bare `tenant`/`Tenant` cannot be used as a pattern because Alembic migrations `0001_initial_schema.py`, `0002_local_simplification.py`, and `cafleet/tests/test_alembic_0002_upgrade.py` — all immutable per H4 — legitimately reference the historical `tenant_id` column name.)
- [x] `mise //:lint`, `mise //:format`, `mise //:typecheck`, `mise //cafleet:test` all pass after every phase.
- [x] Repo-wide "A2A-native" → "A2A-inspired" phrasing unification: `grep -rn 'A2A-native' .` (excluding `design-docs/`, `vendor/`, `admin/node_modules/`, `.venv/`) returns zero matches. The only remaining live A2A-lineage phrasing is "A2A-inspired".
- [x] `cafleet --help` CLI help text reads "A2A-inspired", consistent with README.md:3, CLAUDE.md:17, `.claude/CLAUDE.md:22`, `.claude-plugin/marketplace.json:14`, `.claude-plugin/plugin.json:4`, `cafleet/pyproject.toml:4`.
- [x] `.claude-plugin/plugin.json` `repository` field points to the current repo URL (not `hikyaku`).
- [x] `README.md` Development section no longer says `cd hikyaku`.
- [x] `README.md:200-208` project-structure tree no longer lists the deleted `registry-api.md` / `a2a-operations.md` files.

---

## Background

CAFleet has absorbed six destructive-or-renaming design docs in quick succession:

| # | Slug | Kind |
|---|---|---|
| 0000011 | remove-mcp-server | destructive |
| 0000015 | remove-auth0-local-session-model | destructive |
| 0000019 | project-rename-cafleet | rename (hikyaku → cafleet) |
| 0000020 | tmux-push-notification | additive |
| 0000021 | direct-sqlite-cli | architectural (removed the `/api/v1` REST layer) |
| 0000022 | cafleet-design-doc-skills | additive |

Each of these landed cleanly on its own terms, but collectively they left three kinds of residue:

1. **Dead submodule + stale spec docs** pointing at a world (JSON-RPC over HTTP, `a2a-sdk` Python library, `/api/v1/agents` REST endpoints) that no longer exists because the CLI went direct-SQLite.
2. **A real CLI bug** — `cafleet member create`'s custom prompt path skips the UUID substitution its own SKILL.md documentation promises.
3. **Terminology drift** — "session", "namespace", and in one rule file "tenant" are used interchangeably in prose, even though the schema + CLI settled on "session" exclusively.

This cleanup is scoped to paying these three debts in a single design doc (monolithic per user direction A1) with three shippable phases.

---

## Specification

### Inventory

Every item below carries: (1) **path / surface**, (2) **evidence** (grep count or file reference), (3) **action** (delete / refactor / hold), (4) **phase** (1 / 2 / 3).

#### Category 1 — Delete candidates (Phase 1)

| # | Path / surface | Evidence | Action | Phase |
|---|---|---|---|---|
| D1 | `vendor/A2A/` (git submodule) | `.gitmodules:1-3` defines submodule → `github.com/a2aproject/A2A.git`. No runtime code imports `a2a.*` (verified: `a2a-sdk` is NOT in `cafleet/pyproject.toml:6-14` dependencies). | `git submodule deinit -f vendor/A2A` → edit `.gitmodules` to remove entry → `git rm -rf vendor/A2A` → `rmdir vendor/` if empty. | 1 |
| D2 | `docs/spec/a2a-operations.md` | File header (line 3): "Broker exposes standard A2A endpoints using the `a2a-sdk` Python library. All operations use JSON-RPC 2.0 over HTTP." Contradicts ARCHITECTURE.md ("Direct SQLite access — CLI commands call `broker.py` directly — no HTTP server needed"). | Delete entire file. | 1 |
| D3 | `docs/spec/registry-api.md` | Describes `POST /api/v1/agents`, `DELETE /api/v1/agents/{id}`, etc. (see lines 26-80). No such endpoints exist in `cafleet/src/cafleet/server.py` or `webui_api.py` after 0000021. | Delete entire file. (WebUI-only endpoints are already documented in `docs/spec/webui-api.md`.) | 1 |
| D4 | `.claude/rules/a2a-reference.md` | Entire rule file references `A2A/` and `solace-agent-mesh/` as "related codebases". `solace-agent-mesh/` does not exist anywhere in the repo (verified: `Grep 'solace'` outside `design-docs/` and `vendor/` returns exactly two live hits — this rule file at :14 and `.claude/CLAUDE.md:18`. Both are deleted by D4 + D5.) | Delete entire rule file. | 1 |
| D5 | `.claude/CLAUDE.md` "A2A Protocol Reference" + "Related Codebases" sections (L5-19) | Same stale `A2A/` paths (not even `vendor/A2A/` — the rule predates the submodule move) + dead `solace-agent-mesh/` reference. | Delete both sections. | 1 |
| D6 | `CLAUDE.md:20` AND `.claude/CLAUDE.md:25` — BOTH say "Tenant isolation via shared API key" for design 0000002 (verbatim same bullet, two files) | That design doc's approach was overturned by 0000015 (remove-auth0-local-session-model). "Tenant" is not a word used anywhere in live code. To let the Phase-1 smoke check `grep tenant\|Tenant` succeed with zero matches, the replacement must not re-introduce the word. | Rewrite BOTH files' bullet to read: "Access-control via shared API key (superseded by 0000015 session model)". | 1 |
| D7 | `pyproject.toml:27-35` — `[tool.ty.analysis].allowed-unresolved-imports = ["a2a.*", ...]` | `a2a-sdk` is not a dependency and no `from a2a` import appears in `cafleet/src/`. The entry cannot match anything. | Remove the `"a2a.*"` entry from the list. Leave `pydantic_settings.*`, `starlette.*`, `fastapi.*`, `uvicorn`, `click.*`, `httpx.*`. | 1 |
| D8 | `pyproject.toml:38` — `[tool.ruff].exclude = ["vendor/"]` | Only used to hide the `vendor/A2A/` submodule from ruff. Once D1 lands, there is no `vendor/` to exclude. | Remove the `exclude = ["vendor/"]` entry. | 1 (after D1 lands) |
| D9 | `README.md:200-208` project-structure tree | Tree still lists `registry-api.md` (line 202) and `a2a-operations.md` (line 203) as living files. After D2 + D3 delete them, the tree is stale. | Remove both lines from the tree. | 1 (paired with D2/D3) |

#### Category 2 — Refactor candidates (Phase 2 and 3)

| # | Path / surface | Evidence | Action | Phase |
|---|---|---|---|---|
| R1 | `cafleet/src/cafleet/cli.py:468-486` `_resolve_prompt` | Lines 475-476: `if prompt_argv: return " ".join(prompt_argv)` returns BEFORE `.format()` at line 481. `.claude/skills/cafleet-design-doc-create/SKILL.md:116` promises "`{session_id}` / `{agent_id}` are filled in by `_resolve_prompt`" — true for the default path, false for custom prompts. The Director in this very session had to pre-bake the UUIDs manually because of this. | Refactor: always call `.format()` on the returned string, default path OR custom. Update the docstring accordingly. Add a template-safety note to `.claude/skills/cafleet/SKILL.md` and `.claude/skills/cafleet-design-doc-create/SKILL.md` telling users to double-escape literal `{`/`}` characters or pre-substitute values in custom prompts. See Phase 2 details below. | 2 |
| R2 | `.claude-plugin/plugin.json:8` | `"repository": "https://github.com/himkt/hikyaku"` — misses the 0000019 rename. | Update to `https://github.com/himkt/cafleet` (if that is the canonical URL) or the correct current repo URL. | 2 |
| R3 | `README.md:214-215` | `git clone https://github.com/himkt/hikyaku.git` followed by `cd hikyaku` in the Development section — misses the 0000019 rename. | Update both lines to the current repo URL + `cd cafleet`. | 2 |
| R4 | `cafleet/src/cafleet/cli.py:68` — CLI help text "CAFleet — CLI for the A2A message broker." | Per user answer B3, keep the A2A lineage but unify all repo-wide phrasing to ONE form. Current help text implies the CLI *is* an A2A client, which is no longer true post-0000021. | Rewrite docstring: `"""CAFleet — CLI for the A2A-inspired message broker."""`. Paired with R12 below so the `cafleet --help` output matches README.md:3 and every other user-visible metadata surface. | 2 |
| R5 | `cafleet/src/cafleet/db/models.py:4` module docstring "indexed/queried fields to columns and stores opaque A2A payloads" | Same lineage-tone issue. `task_json` / `agent_card_json` blobs are CAFleet-shaped, not `a2a-sdk` objects. | Rewrite to "stores opaque A2A-inspired task + agent-card payloads as JSON TEXT". | 2 |
| R6 | *(folded into R12 — do not handle separately)* | *(see R12)* | *(see R12)* | 2 |
| R12 | Repo-wide "A2A-native" → "A2A-inspired" unification — 6 live locations | Grep `'A2A-native\|A2A-inspired\|A2A-aligned'` outside `design-docs/` and `vendor/` (verified): `A2A-native` appears at README.md:3, CLAUDE.md:17, `.claude/CLAUDE.md:22`, `.claude-plugin/marketplace.json:14`, `.claude-plugin/plugin.json:4`, `cafleet/pyproject.toml:4`. `A2A-inspired` appears **nowhere** live. Without this row, Phase 2's R4 change would literally contradict README.md's first sentence. | Rewrite all 6 occurrences from "A2A-native" to "A2A-inspired" in the same Phase 2 commit as R4/R5, so the `cafleet --help` output, Python package description, plugin description, marketplace description, and all CLAUDE.md headers agree on one phrasing. | 2 |
| R7 | `docs/spec/data-model.md:3` claims core data structures "use types defined by the A2A specification via `a2a-sdk` Pydantic models" | `a2a-sdk` is not a dependency. Line 7 also claims the runtime engine is "SQLAlchemy 2.x with the `aiosqlite` async driver" — but `cafleet/src/cafleet/db/engine.py` uses the sync `pysqlite` driver (`get_sync_engine` / `get_sync_sessionmaker`). Two factual errors in the first eight lines. | Rewrite opening paragraphs to describe CAFleet's actual task/agent-card JSON shape (internal, A2A-inspired, not SDK-backed) and the sync `pysqlite` driver. Leave the SQL schema tables untouched — they remain accurate. | 2 |
| R8 | `docs/spec/data-model.md:32, 52, 55, 173` — column notes saying "A2A `AgentCard` blob, stored verbatim", "A2A `TaskState` enum value", "normal A2A traffic" | Same source-of-truth issue — there is no external A2A library backing these blobs. | Rewrite to "AgentCard-shaped blob (A2A-inspired, internal schema)" etc. | 2 |
| R9 | Naming drift — `namespace` used to mean `session` (re-greped 2026-04-14, exact per-line truth below) | Grep `'namespace'` in tracked non-vendored sources: **README.md** — 7 hits at lines 7, 12, 17, 40, 68, 116, 132. **ARCHITECTURE.md** — 4 lines with 7+ occurrences: line 3 ("non-secret namespace"), line 30 ("namespace boundary" / "one namespace" / "namespace routing"), line 32 ("non-secret namespace identifier" / "Sessions are namespaces for tidiness"), line 45 ("session namespace CRUD"). **CLAUDE.md** — 1 hit at line 24 ("session for namespace CRUD"). **.claude/CLAUDE.md** — 1 hit at line 29 (mirror of CLAUDE.md:24). **.claude/skills/cafleet/SKILL.md** — 0 hits. **docs/spec/cli-options.md** — 2 hits at lines 25 ("Session namespace UUID") and 81 ("manages session namespaces"). **docs/spec/webui-api.md** — 0 hits. | Replace every session-meaning "namespace" → "session" in prose across README.md, ARCHITECTURE.md, CLAUDE.md, .claude/CLAUDE.md, docs/spec/cli-options.md. Keep the dual-name form "session (namespace)" in exactly ONE location: `docs/spec/cli-options.md:25` global-options table. `docs/spec/cli-options.md:81` ("session namespaces") is rewritten to plain "sessions". Files with 0 hits (`.claude/skills/cafleet/SKILL.md`, `docs/spec/webui-api.md`) have no task. DO NOT rename any Python identifier, SQL column, CLI flag, or env var. | 3 |
| R10 | Naming drift — `Tenant isolation` as legacy prose synonym | Grep `'Tenant isolation'` outside `design-docs/` (verified): 2 live hits — `CLAUDE.md:20` AND `.claude/CLAUDE.md:25` (Reviewer-corrected line number; previously mis-cited as :14). **The bare `tenant`/`Tenant` pattern is NOT a valid scope for this row**: Alembic migration `0001_initial_schema.py` (5 hits), `0002_local_simplification.py` (5 hits), and `cafleet/tests/test_alembic_0002_upgrade.py` (14 hits) legitimately reference the historical `tenant_id` column name, and those files are immutable per H4. | Both bullets are rewritten by D6. R10 carries no additional work. The Phase 1 smoke check uses the exact phrase `Tenant isolation` (not bare `tenant\|Tenant`) so the immutable Alembic + test files are not false positives. | 1 (piggybacks on the updated D6) |
| R11 | `cafleet/src/cafleet/cli.py` is 800+ lines with db, session, client, member subgroups all in one file | Module layering is tolerable today (single click root), but further CLI growth would justify splitting into `cli/db.py`, `cli/session.py`, `cli/member.py`, and a slim `cli/__init__.py` that registers each subgroup. | **Do not split in this pass.** Note the refactor opportunity in Hold-with-reason so a future design doc can pick it up. | — (Hold) |

#### Category 3 — Hold-with-reason (no action this cycle)

| # | Path / surface | Reason to hold |
|---|---|---|
| H1 | Historical design docs 0000001-0000023 that contain `hikyaku` or `A2A` wording | User answer A2: leave as historical record. The design-docs/ directory is a commit log, not living documentation. |
| H2 | DB schema names (`Session` model, `session_id` columns, `sessions` table) | User answer C2: rename nothing at the schema level. The CLI flag `--session-id` is already canonical. |
| H3 | Python identifiers and CLI flag names | Same as H2 — naming unification is **prose-only** in this pass. |
| H4 | `cafleet/tests/` obsolete tests | Recon confirmed: no tests exist for removed features (MCP, Auth0, a2a-sdk imports, `/api/v1/*` REST). The current suite covers only live surfaces — tests around broker, CLI, alembic, tmux, coding_agent, output. No deletions warranted. |
| H5 | Splitting `cli.py` into submodules | R11 — out of scope. Flagged for a future design doc. |
| H6 | Splitting `broker.py` (currently the sole data-access layer) | `broker.py` is intentionally the single module — the architectural contract in ARCHITECTURE.md is "broker.py is THE data access layer". Splitting it would fight that contract for no concrete benefit today. |
| H7 | Rule duplication between `.claude/rules/` (project) and `~/.claude/rules/` (user-global) | By design: `.claude/rules/git-workflow.md:1-3` explicitly states it **overrides** the user-global rule. The pattern is documented and working. Do not consolidate. |
| H8 | Admin/WebUI component refactors | User answer A3: WebUI redesign is out of scope. Grep confirms admin/ has zero `hikyaku` or `A2A` references (`Grep 'hikyaku\|Hikyaku'` in `admin/` excluding `node_modules/` returns empty; same for `a2a`). Nothing to fix in admin/ in this pass. |

### Phased Plan

Each phase is independently shippable (can be a standalone PR). Phase N+1 depends on Phase N only in the sense that they target the same design doc; nothing in Phase 2's code change requires Phase 1's deletions to have landed.

#### Phase 1 — Low-risk deletions (items D1-D9, R10 via updated D6)

**Blast radius**: removes a git submodule, three doc files, one rule file, two stale CLAUDE.md subsections, one stale pyproject entry, and two stale tree lines in README.md. Zero runtime code is modified. Zero tests change. Zero permissions patterns change.

Tasks in Implementation → Step 1 below.

#### Phase 2 — Code fix + repo-wide lineage-tone unification (R1-R8 + R12)

**Blast radius**: ONE code file (`cafleet/src/cafleet/cli.py`), ONE new test file, three SKILL.md edits, three docstring/help-text edits, six "A2A-native → A2A-inspired" prose/metadata edits across README.md / CLAUDE.md / .claude/CLAUDE.md / marketplace.json / plugin.json / cafleet/pyproject.toml (R12), two hikyaku-rename fixes (R2/R3), and data-model.md accuracy fixes (R7/R8). Changes are additive at the test level and tone-only at the prose/metadata level.

Tasks in Implementation → Step 2 below.

#### Phase 3 — Naming unification (R9)

**Blast radius**: prose-only. No source file under `cafleet/src/` is touched. No SQL column, CLI flag, or Python identifier is renamed.

Tasks in Implementation → Step 3 below.

### Risks and Non-Goals

**Risks**:

| Risk | Mitigation |
|---|---|
| `git submodule deinit` mistakenly nuking local uncommitted work inside `vendor/A2A/` | Phase 1 runbook: `git status -s vendor/A2A` MUST be clean before deinit. If it reports anything, abort and surface to the user. |
| Removing `.claude/rules/a2a-reference.md` breaks an existing `permissions.allow` pattern that references the file | Search `.claude/settings.json` + `.claude/settings.local.json` for `a2a-reference`. If found, update or remove that allow-pattern. Verified during drafting: current `settings.json` does NOT reference this file. |
| The `_resolve_prompt` fix breaks callers that already pre-bake UUIDs into custom prompts (today's Director workaround) | Pre-baked prompts contain literal UUIDs, not `{...}` placeholders. `.format()` on a string with no `{...}` placeholders is a no-op. Existing callers are unaffected. Only NEW risk is literal `{`/`}` in a custom prompt (e.g. a JSON snippet). The SKILL.md template-safety note + regression test for a curly-brace-literal case cover it. |
| Naming unification bulk edit accidentally touches the DB schema or a CLI flag name | Phase 3 is guarded by a grep-whitelist: only replace "namespace" inside .md/.mdx prose files AND only when it refers to a session. SQL-schema files (`db/models.py`, `alembic/versions/*.py`) and CLI (`cli.py`) are NOT in the edit set. |

**Non-Goals** (explicitly out of scope):

- New features of any kind.
- Any DB schema change (no new columns, no renames, no new indexes).
- WebUI redesign or any component-design refactor in `admin/`.
- Renaming Python identifiers, CLI flags, env vars, or SQL column names.
- Splitting `cli.py` or `broker.py` into submodules (flagged as H5 / H6 for a future design doc).
- Consolidating `.claude/rules/` with `~/.claude/rules/` (H7 — by design).
- Editing historical design docs 0000001-0000023 (H1 — historical record).
- Replacing existing tests (H4 — the suite is tight as-is).

### Verification Strategy

**Standard checks** (run after every phase):

| Task | Purpose |
|---|---|
| `mise //:lint` | Workspace-wide ruff check. |
| `mise //:format` | Workspace-wide ruff format check. |
| `mise //:typecheck` | Workspace-wide ty typecheck. |
| `mise //cafleet:test` | `pytest` suite for the `cafleet` package. |
| `mise //admin:lint` | WebUI lint (included for safety even though Phase 1-3 do not touch `admin/`). |

**Pre-delete cross-reference scan** (Phase 1, before any file is deleted — catches stale references without relying on post-delete smoke):

```
grep -rn 'docs/spec/a2a-operations\.md\|docs/spec/registry-api\.md' . --exclude-dir=design-docs --exclude-dir=vendor --exclude-dir=node_modules --exclude-dir=.venv
```

Known pre-existing references (as of 2026-04-14 drafting): README.md:202-203 (project-structure tree — handled by D9). Any other match surfaced by the scan must get an explicit edit task added to Phase 1 before D2 / D3 delete the files.

**Phase-specific smoke checks**:

| Phase | Smoke check | Pass criterion |
|---|---|---|
| 1 | `git submodule status` | No entry for `vendor/A2A` remains. |
| 1 | `ls vendor/` (only if the directory still exists) | Either the directory is gone OR it is empty. |
| 1 | `grep -rn 'solace-agent-mesh' .claude/ CLAUDE.md` (exclude `.claude/settings.json`) | Zero matches. |
| 1 | `grep -rn 'docs/spec/a2a-operations.md\|docs/spec/registry-api.md' . --exclude-dir=design-docs --exclude-dir=vendor` | Zero matches after D9 lands. |
| 1 | `grep -rn 'Tenant isolation' . --exclude-dir=design-docs --exclude-dir=vendor` | Zero matches after the updated D6 lands (the replacement uses "Access-control"). Note: the bare `tenant\|Tenant` pattern is NOT suitable as a smoke check because Alembic migrations and the `test_alembic_0002_upgrade.py` regression test legitimately reference the historical `tenant_id` column name (~23 live matches across those three immutable files, protected by H4). Only the exact phrase `Tenant isolation` reliably isolates the two live CLAUDE.md bullets that D6 / 1.11 / 1.12 rewrite. |
| 2 | `cafleet --help` (delegate to a teammate with CLI permission, or ask the user to run it) | Top-line text reads "A2A-inspired", consistent with the post-R12 README.md:3 opening sentence. |
| 2 | `grep -rn 'A2A-native' . --exclude-dir=design-docs --exclude-dir=vendor --exclude-dir=node_modules --exclude-dir=.venv` | Zero matches after Phase 2 (R12 rewrites all 6 live occurrences). |
| 2 | `cafleet --session-id <sid> member create --agent-id <did> --name Test --description "template-safety smoke" -- "my agent id is {agent_id}"` | The new pane's prompt contains the substituted UUID, not the literal `{agent_id}` string. |
| 2 | `cafleet --session-id <sid> member create --agent-id <did> --name Test --description "brace-literal smoke" -- "data is {{not a placeholder}} closed"` | The new pane's prompt contains the literal string `data is {not a placeholder} closed` — single curly braces on the receiving end, confirming the doubled-brace escape path works. |
| 2 | `cat .claude-plugin/plugin.json \| python -c 'import json,sys; d=json.load(sys.stdin); assert "hikyaku" not in d["repository"]'` (or equivalent Read + visual check) | Repository URL no longer contains `hikyaku`. |
| 2 | New test file `cafleet/tests/test_cli_member.py` passes | Covers: (a) default prompt gets UUIDs substituted (existing behaviour), (b) custom prompt with `{agent_id}` gets UUIDs substituted (new behaviour, regression for R1), (c) custom prompt with NO placeholders passes through unchanged, (d) custom prompt with doubled `{{...}}` escapes collapses to single curly braces on output (risk-row mitigation test). |
| 3 | `grep -rn 'namespace' README.md ARCHITECTURE.md CLAUDE.md .claude/CLAUDE.md docs/spec/cli-options.md` | Exactly ONE match remaining: `docs/spec/cli-options.md:25` with the post-edit phrasing `"Session UUID (namespace identifier)"` (see task 3.5). All other occurrences (README.md's 7, ARCHITECTURE.md's 7+, CLAUDE.md's 1, .claude/CLAUDE.md's 1, cli-options.md's second hit at the "manages session namespaces" line) are rewritten to "session". |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1 — Phase 1: Low-risk deletions

- [x] 1.1 Run the pre-delete cross-reference scan from the Verification Strategy section. Record every file that mentions `docs/spec/a2a-operations.md` or `docs/spec/registry-api.md` so that Step 1.X edits cover all of them. Known matches as of drafting: README.md:202-203 (handled by 1.15). Abort and reopen this design doc if any new surface is found. <!-- completed: 2026-04-15T12:00 -->
- [x] 1.2 Run `git status -s vendor/A2A` and abort if output is non-empty. <!-- completed: 2026-04-15T12:00 -->
- [x] 1.3 `git submodule deinit -f vendor/A2A`. <!-- completed: 2026-04-15T12:00 -->
- [x] 1.4 Edit `.gitmodules` — remove the `[submodule "vendor/A2A"]` stanza; if `.gitmodules` becomes empty, `git rm .gitmodules`. <!-- completed: 2026-04-15T12:00 -->
- [x] 1.5 `git rm -rf vendor/A2A`. <!-- completed: 2026-04-15T12:00 -->
- [x] 1.6 If `vendor/` directory is now empty, `rmdir vendor/`. <!-- completed: 2026-04-15T12:00 -->
- [x] 1.7 Delete `docs/spec/a2a-operations.md`. <!-- completed: 2026-04-15T12:00 -->
- [x] 1.8 Delete `docs/spec/registry-api.md`. <!-- completed: 2026-04-15T12:00 -->
- [x] 1.9 Delete `.claude/rules/a2a-reference.md`. <!-- completed: 2026-04-15T12:00 -->
- [x] 1.10 Edit `.claude/CLAUDE.md` — delete the "A2A Protocol Reference" section (the heading and its bullet list describing `A2A/specification/a2a.proto` etc.) AND the "Related Codebases" section (the heading and its `A2A/` + `solace-agent-mesh/` bullets). Currently at lines 5-19 but use the section headers as the locator since subsequent tasks will depend on the post-deletion layout. <!-- completed: 2026-04-15T12:00 -->
- [x] 1.11 Edit `CLAUDE.md:20` — rewrite the 0000002 bullet from "Tenant isolation via shared API key" to "Access-control via shared API key (superseded by 0000015 session model)". <!-- completed: 2026-04-15T12:00 -->
- [x] 1.12 Edit `.claude/CLAUDE.md` — find the bullet line currently saying `"Design document: design-docs/0000002-access-control/design-doc.md — Tenant isolation via shared API key (Status: Complete)"` and rewrite it to `"Design document: design-docs/0000002-access-control/design-doc.md — Access-control via shared API key (superseded by 0000015 session model) (Status: Complete)"`. (Evidence cell cites pre-Phase-1 line as :25; content-based locator is used here because 1.10 deletes 15 preceding lines in the same phase and would shift any line-number cite.) <!-- completed: 2026-04-15T12:00 -->
- [x] 1.13 Edit `pyproject.toml` — remove the `"a2a.*"` entry from `[tool.ty.analysis].allowed-unresolved-imports`. <!-- completed: 2026-04-15T12:00 -->
- [x] 1.14 Edit `pyproject.toml` — remove `exclude = ["vendor/"]` from `[tool.ruff]` (only after 1.6 confirms `vendor/` is gone). <!-- completed: 2026-04-15T12:00 -->
- [x] 1.15 Edit `README.md:200-208` project-structure tree — delete the two lines listing `registry-api.md` and `a2a-operations.md` (D9). Keep the enclosing `spec/` subtree structure; only the deleted filenames go away. <!-- completed: 2026-04-15T12:00 -->
- [x] 1.16 Run `mise //:lint`, `mise //:format`, `mise //:typecheck`, `mise //cafleet:test`. All must pass. <!-- completed: 2026-04-15T12:05 -->
- [x] 1.17 Run Phase 1 smoke checks from the Verification Strategy table. <!-- completed: 2026-04-15T12:05 -->

### Step 2 — Phase 2: `_resolve_prompt` fix + repo-wide lineage unification

- [x] 2.1 Edit `cafleet/src/cafleet/cli.py:468-486` `_resolve_prompt` — when `prompt_argv` is non-empty, join into a template string, then call `.format(session_id=..., agent_id=..., director_name=..., director_agent_id=...)` on the joined string (same kwargs as the default branch). Keep the existing director lookup + UsageError on missing director. <!-- completed: 2026-04-15T12:30 -->
- [x] 2.2 Add `cafleet/tests/test_cli_member.py` — FOUR test cases against `_resolve_prompt` directly: (a) default path substitutes UUIDs, (b) `prompt_argv=("message","for","{agent_id}")` substitutes UUIDs, (c) `prompt_argv=("no","placeholders","here")` passes through unchanged, (d) `prompt_argv=("data","is","{{not","a","placeholder}}","closed")` collapses doubled braces to single braces and does NOT attempt placeholder substitution on the inner tokens (covers the Risk-row mitigation for literal-brace JSON snippets). <!-- completed: 2026-04-15T12:30 -->
- [x] 2.3 Edit `.claude/skills/cafleet/SKILL.md` — add a "Template safety" note under `Member Create`: custom prompts go through `str.format()`, so literal `{`/`}` must be doubled (`{{` / `}}`) or the value pre-substituted before calling `member create`. <!-- completed: 2026-04-15T12:30 -->
- [x] 2.4 Edit `.claude/skills/cafleet-design-doc-create/SKILL.md:116` — update the surrounding copy to confirm the custom-prompt path now substitutes `{session_id}` and `{agent_id}` too. Add a template-safety cross-reference to `cafleet/SKILL.md`. <!-- completed: 2026-04-15T12:30 -->
- [x] 2.5 Edit `cafleet/src/cafleet/cli.py:68` — rewrite the CLI docstring to read `"""CAFleet — CLI for the A2A-inspired message broker."""`. (R4) <!-- completed: 2026-04-15T12:30 -->
- [x] 2.6 Edit `cafleet/src/cafleet/db/models.py:4` — rewrite the module docstring from "opaque A2A payloads" to "opaque A2A-inspired task + agent-card payloads as JSON TEXT". (R5) <!-- completed: 2026-04-15T12:30 -->
- [x] 2.7 Edit `docs/spec/data-model.md:3-7` — remove the `a2a-sdk` claim; rewrite to describe CAFleet's internal A2A-inspired shape. Change line 7's `aiosqlite async driver` reference to the sync `pysqlite` driver with a pointer to `cafleet/src/cafleet/db/engine.py`. (R7) <!-- completed: 2026-04-15T12:30 -->
- [x] 2.8 Edit `docs/spec/data-model.md:32, 52, 55, 173` — replace "A2A `AgentCard` blob / `TaskState` enum value / normal A2A traffic" with "AgentCard-shaped blob / TaskState enum value / normal traffic" (or the agreed lineage-marker phrasing). (R8) <!-- completed: 2026-04-15T12:30 -->
- [x] 2.9 Edit `.claude-plugin/plugin.json:8` — update `repository` from `github.com/himkt/hikyaku` to the canonical current repo URL. (R2) <!-- completed: 2026-04-15T12:30 -->
- [x] 2.10 Edit `README.md:214-215` — update `git clone` URL and the `cd <dir>` line to the canonical current repo name. (R3) <!-- completed: 2026-04-15T12:30 -->
- [x] 2.11 **R12 — repo-wide "A2A-native" → "A2A-inspired" unification (6 files, 6 edits)**: <!-- completed: 2026-04-15T12:30 -->
  - [x] 2.11.1 `README.md:3` — "A2A-native message broker" → "A2A-inspired message broker". <!-- completed: 2026-04-15T12:30 -->
  - [x] 2.11.2 `CLAUDE.md:17` — "A2A-native message broker + agent registry" → "A2A-inspired message broker + agent registry". <!-- completed: 2026-04-15T12:30 -->
  - [x] 2.11.3 `.claude/CLAUDE.md` — find the line currently saying `"A2A-native message broker + agent registry for coding agents."` (same bullet as 2.11.2 but in the mirrored `.claude/` copy; was line :22 pre-Phase-1, shifts to ~:7 after 1.10) and change `"A2A-native"` → `"A2A-inspired"`. <!-- completed: 2026-04-15T12:30 -->
  - [x] 2.11.4 `.claude-plugin/marketplace.json:14` — "A2A-native message broker CLI" → "A2A-inspired message broker CLI". <!-- completed: 2026-04-15T12:30 -->
  - [x] 2.11.5 `.claude-plugin/plugin.json:4` — "A2A-native message broker CLI and design document orchestration skills for coding agents." → "A2A-inspired message broker CLI and design document orchestration skills for coding agents." <!-- completed: 2026-04-15T12:30 -->
  - [x] 2.11.6 `cafleet/pyproject.toml:4` — `description = "A2A-native message broker and agent registry for coding agents"` → `description = "A2A-inspired message broker and agent registry for coding agents"`. <!-- completed: 2026-04-15T12:30 -->
- [x] 2.12 Run `mise //:lint`, `mise //:format`, `mise //:typecheck`, `mise //cafleet:test`. All must pass (new test from 2.2 must pass). <!-- completed: 2026-04-15T12:35 -->
- [ ] 2.13 Run Phase 2 smoke checks from the Verification Strategy table. Delegate `cafleet --help` / `cafleet member create` smoke invocations to a teammate that has CLI permission, or ask the user to run them. Do NOT run them directly. The `grep A2A-native` zero-match assertion is self-runnable. <!-- completed: -->

### Step 3 — Phase 3: Naming unification (prose-only, per re-greped line truth)

Files with ZERO hits — `.claude/skills/cafleet/SKILL.md` and `docs/spec/webui-api.md` — have no task in Phase 3. The dual-name exception survives in exactly ONE location: `docs/spec/cli-options.md:25`. Everything else is rewritten.

- [x] 3.1 Edit `README.md` — rewrite 7 occurrences at lines 7, 12, 17, 40, 68, 116, 132. Do NOT attempt a single sed replace — apply per-line reasoning so that phrases like "session_id namespace" become "session_id", "namespace boundary" becomes "session boundary", "session namespace" becomes "session", etc. <!-- completed: 2026-04-15T12:45 -->
- [x] 3.2 Edit `ARCHITECTURE.md` — rewrite occurrences at lines 3, 30, 32, 45 (7+ total occurrences across those lines): line 3 "non-secret namespace" → "non-secret session"; line 30 "namespace boundary" → "session boundary", "form one namespace" → "form one session", "namespace routing" → "session routing"; line 32 "non-secret namespace identifier" → "non-secret session identifier", "Sessions are namespaces for tidiness" → "Sessions are partitions for tidiness"; line 45 "session namespace CRUD" → "session CRUD". <!-- completed: 2026-04-15T12:45 -->
- [x] 3.3 Edit `CLAUDE.md:24` — rewrite "session for namespace CRUD" to "session for session CRUD" (or the least-redundant phrasing decided during Phase-3 review; "session CRUD" alone is preferred). <!-- completed: 2026-04-15T12:45 -->
- [x] 3.4 Edit `.claude/CLAUDE.md` — find the line currently saying `"Unified CLI command: cafleet (with db init for schema management, session for namespace CRUD, and all agent/messaging commands)"` (mirror of 3.3; was line :29 pre-Phase-1, shifts to ~:14 after 1.10) and apply the same "session for namespace CRUD" → "session for session CRUD" (or preferred "session CRUD") rewrite as 3.3. <!-- completed: 2026-04-15T12:45 -->
- [x] 3.5 Edit `docs/spec/cli-options.md:25` — keep the dual-name form ONCE at this location: change "Session namespace UUID" to "Session UUID (namespace identifier)". This is the sole surviving dual-name spot. <!-- completed: 2026-04-15T12:45 -->
- [x] 3.6 Edit `docs/spec/cli-options.md:81` — change "manages session namespaces" to "manages sessions". <!-- completed: 2026-04-15T12:45 -->
- [x] 3.7 Re-run the grep assertion: `grep -rn 'namespace' README.md ARCHITECTURE.md CLAUDE.md .claude/CLAUDE.md docs/spec/cli-options.md`. Exactly ONE match must remain — the dual-name form at `docs/spec/cli-options.md:25`. Abort and fix if any other match surfaces. <!-- completed: 2026-04-15T12:45 -->
- [x] 3.8 Run `mise //:lint`, `mise //:format`, `mise //:typecheck`, `mise //cafleet:test`. All must pass (typecheck + test are effectively no-ops for prose-only changes, but run them to rule out accidental file corruption). <!-- completed: 2026-04-15T12:50 -->
- [x] 3.9 Run Phase 3 smoke checks from the Verification Strategy table. <!-- completed: 2026-04-15T12:50 -->

### Step 4 — Commit + review sequence

- [ ] 4.1 Commit Phase 1 as a single commit: `chore: remove vendor/A2A submodule and dead A2A-era spec docs`. <!-- completed: -->
- [ ] 4.2 Commit Phase 2 as a single commit: `fix(cli): apply str.format to custom member prompts and align A2A-lineage copy`. <!-- completed: -->
- [ ] 4.3 Commit Phase 3 as a single commit: `docs: unify "session" vs "namespace" wording in prose`. <!-- completed: -->
- [ ] 4.4 Commit the design doc at the end of the sequence: `docs: add design doc 0000024-cleanup-and-refactor`. (Project override allows design-docs/ to be committed in this project — see `.claude/rules/git-workflow.md`.) <!-- completed: -->
- [ ] 4.5 Open PR covering all four commits. Post-merge, mark this design doc's Status → Complete. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-14 | Initial draft (v1). |
| 2026-04-14 | v2 — Reviewer revisions. Fixed R9 per-line evidence (re-greped): README.md 7 hits not 2, ARCHITECTURE.md 4 lines not 0, `.claude/skills/cafleet/SKILL.md` 0 hits not "multiple", docs/spec/cli-options.md 2 lines not 1, docs/spec/webui-api.md 0 hits not 1. Fixed D6 to rewrite BOTH `CLAUDE.md:20` and `.claude/CLAUDE.md:25` with "Access-control" phrasing (removing the literal word "Tenant" so the Phase-1 smoke check is reachable). Added D9 (README.md project-structure tree cleanup) + pre-delete cross-reference scan. Added R12 (repo-wide "A2A-native" → "A2A-inspired" unification across 6 files) to resolve lineage-tone contradiction flagged in Reviewer I3; folded R6 into R12. Added test case (d) in 2.2 for literal-curly-brace escape. Clarified dual-name exception survives at ONE location only (cli-options.md:25). Fixed `.gitmodules` cite to :1-3 and `.claude/CLAUDE.md` line cite to :25. |
| 2026-04-14 | v3 — Reviewer revisions. (I5) Tightened the `tenant`/`Tenant` smoke-check pattern and success criterion to the exact phrase `Tenant isolation`; rationale now documents the three immutable Alembic migrations + regression test that carry legitimate `tenant_id` references and must not be false positives. Updated R10 row accordingly. (I6) Replaced stale post-Phase-1 line-number cites with content-based locators in tasks 1.10, 1.12, 2.11.3, 3.4 — so that when 1.10 deletes 15 lines from `.claude/CLAUDE.md` the downstream tasks still resolve to the correct bullet. (P4) Updated Phase-3 smoke-check description text to reference the post-edit dual-name phrasing `"Session UUID (namespace identifier)"` from task 3.5. |
| 2026-04-15 | v3 approved by user. Status → Approved. Last Updated refreshed. Progress set to 0/50 — ready for implementation. |
| 2026-04-15 | Implementation complete. All 3 phases shipped — Phase 1 c8afc8a (vendor/A2A submodule + dead spec docs removed), Phase 2 e75ae76 + 3c05fc9 (_resolve_prompt str.format fix with 4 regression tests + repo-wide A2A-native → A2A-inspired lineage unification), Phase 3 ba67e3e (session vs namespace prose unification). All 11 Success Criteria PASS, mise //:lint / //:format / //:typecheck pass, 271 pytest tests pass. Status → Complete. |
