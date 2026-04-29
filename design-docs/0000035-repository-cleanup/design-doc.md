# Repository cleanup — remove obsolete files, stale skill content, dead pyproject config

**Status**: Approved
**Progress**: 23/25 tasks complete
**Last Updated**: 2026-04-29

## Overview

After 34 design-doc cycles the CAFleet repository has accumulated a small set of obsolete artifacts left behind by feature removals (0000007 API keys, 0000011 MCP server, 0000015 Auth0, 0000018 multi-runner backends, 0000034 bash-via-Director). This design records every concrete candidate with line-level evidence, classifies each as delete / update / keep, and ships the cleanup as a single PR in the project-mandated order: docs → skills → source-config → tests → lockfile → verification.

## Success Criteria

- [x] The grep set in §"Acceptance grep set" below returns zero hits outside `design-docs/`, alembic migration sources/tests, and admin lockfiles.
- [x] All sentinel-style "removed-flag-now-errors" / "removed-import-now-fails" tests are gone (`TestCodexConstantRemoved`, `test_no_bash_flag_no_longer_parses`, `test_allow_bash_flag_no_longer_parses`, `TestCodingAgentFlagRemoved`).
- [x] `cafleet/pyproject.toml` `[tool.ty.analysis].allowed-unresolved-imports` no longer references modules that the source tree never imports (specifically `httpx.*`).
- [x] `admin/bun.lock` no longer pins `@auth0/*` packages (regenerated from current `admin/package.json`).
- [x] User-facing docs no longer "advertise the deletion" of removed features. Specifically: no `No Auth0` / `No bearer tokens, no API keys` sentences in `ARCHITECTURE.md` or `docs/spec/webui-api.md`; no `## Removed CLI Options` section in `docs/spec/cli-options.md`; no `migrated sessions reuse the original api_key_hash value` clauses in `docs/spec/data-model.md` or `docs/spec/cli-options.md`.
- [x] `skills/cafleet/SKILL.md` line 448 no longer carries the apology meta-line `"The CLI never inspects placement.coding_agent."`.
- [x] An explicit decision note is recorded in this document: `--permission-mode dontAsk` is the canonical member-spawn discipline; bash-via-Director (`cafleet member send-input --bash`) remains an opt-in escape hatch, and `--disallowedTools "Bash"` is not used.
- [x] `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test`, `mise //admin:lint`, and `mise //admin:build` all pass after the cleanup.

---

## Background

### Removed features and their authorizing design docs

| Removed feature | Authorizing design | Status |
|---|---|---|
| API-key auth | 0000007 (introduced) → 0000015 (removed) | Complete |
| MCP server | 0000011 | Complete |
| Auth0 / bearer tokens | 0000015 | Complete |
| Multi-runner backends (`--coding-agent`, `CODEX`, `CODING_AGENTS`, `get_coding_agent`) | 0000018 (introduced) → 0000034 (removed) | Complete |
| `--no-bash` / `--allow-bash` / `--disallowedTools "Bash"` member-spawn discipline | 0000034 R11→R12 (revised in-place to `--permission-mode dontAsk`) | Complete |

### Cleanup philosophy

`.claude/rules/removal.md` governs. The repository should read as if removed features never existed — no deprecation notices, no "for history" pointers, no sentinel-style "deprecated → error" tests, and no `Sentences like 'pre-existing X rows are preserved for forensic visibility'` in user-facing surfaces. Migration code, alembic versions, and the design-doc historical record are the canonical history; user-facing docs, skills, and tests describe only the current state.

### Anti-scope (not addressed by this design)

- No project rename, no architectural rewrite, no CLI surface changes.
- Historical `design-docs/*/design-doc.md` directories are not deleted. One narrow edit is proposed for `design-docs/0000034-member-bash-via-director/design-doc.md` — see §"Specification → 0000034 reconciliation".
- `.git/`, `.gitignore`, `.mise.toml`, `pyproject.toml` build/dependency entries are not touched except for the dead `httpx.*` `allowed-unresolved-imports` line.
- Bulk reformatting / mass renames purely for style. `cafleet/src/cafleet/coding_agent.py` is kept as-is even though only `CLAUDE` remains — collapsing a single-config-stub module into its caller is a style refactor, not a removal.
- Schema-level removal of the `agent_placements.coding_agent` column is out of scope (the column has live values; removal would be a data-model change).

---

## Specification

### Concrete candidates with classification

Evidence rows below were verified by direct file reads. Line numbers are at the time of drafting (2026-04-29) and are advisory; the executor must re-grep before editing.

#### Documentation surface

| Path | Line | Issue | Classification | Action |
|---|---|---|---|---|
| `ARCHITECTURE.md` | 32 | Sentence `"No bearer tokens, no API keys, no Auth0. The session_id is a non-secret session identifier. Sessions are partitions for tidiness, not security boundaries."` advertises the deletion of Auth0 / API keys. | UPDATE | Delete only the leading `"No bearer tokens, no API keys, no Auth0. "` clause (29 characters including the trailing space). Keep the existing trailing sentence verbatim: `"The session_id is a non-secret session identifier. Sessions are partitions for tidiness, not security boundaries."` (Per Reviewer P2: do not introduce a new "not an authentication token" formulation — that is itself a soft what-it-isn't clause.) |
| `docs/spec/webui-api.md` | 13 | Sentence `"No server-side session. No Auth0. The SPA manages the active session_id client-side via hash-based routing."` advertises Auth0 removal AND conflicts with the preceding header note that the backend verifies `X-Session-Id` against the `sessions` table. | UPDATE | Replace with `"No server-side session cookies. The SPA stores the active session_id client-side via hash-based routing and sends it in the X-Session-Id header on each request."` (drops the `"No Auth0. "` clause AND clarifies cookies vs server-side state per Copilot R1 review). |
| `docs/spec/cli-options.md` | 47–57 | Subsection `## Removed CLI Options` enumerates `--url` / broker-URL env var, `--api-key` flag, removed session-id env var, removed agent-id env var, and `cafleet env` subcommand. Trailing sentence on line 57 (`"These removals keep secrets out of shell history and let permissions.allow patterns match every invocation literally."`) compounds the deletion advertisement. Direct violation of `removal.md`: "Flag rows in CLI tables documenting removed flags ('--coding-agent — deprecated')". | DELETE | Remove the entire `## Removed CLI Options` subsection together with its trailing rationale sentence (lines 47–57 inclusive). The current `## Global Options` section (lines 18–45) and `## Agent ID (--agent-id)` section (line 59 onward) remain untouched. |
| `docs/spec/cli-options.md` | 25 | `--session-id <id>` row contains parenthetical `"... new sessions get a UUIDv4, migrated sessions reuse a 64-char hex value)"` — the "migrated sessions" half is a forensic-visibility statement about a removed feature. | UPDATE | Replace the parenthetical's contents with `"opaque string; new sessions receive a UUIDv4"`. Keep the surrounding "Also called the namespace identifier..." sentence unchanged. |
| `docs/spec/data-model.md` | 15 | `sessions.session_id` row Notes column: `"Opaque string. New sessions receive a UUIDv4; migrated sessions reuse the original api_key_hash value (64-char hex)."` — the "migrated sessions reuse the original api_key_hash value" half is a `removal.md`-forbidden forensic-visibility sentence. | UPDATE | Rewrite the Notes cell to: `"Opaque string. New sessions receive a UUIDv4."`. |

#### Skills surface

| Path | Line | Issue | Classification | Action |
|---|---|---|---|---|
| `skills/cafleet/SKILL.md` | 448 | Apology meta-line `"The CLI never inspects placement.coding_agent."` exists solely to explain a residual schema field. | UPDATE | Delete the line. The four JSON examples on lines 291 / 367 / 551 / 558 keep the `coding_agent` field because the column is real and stores `"claude"` or `"unknown"`. |
| `skills/cafleet/SKILL.md` | 558 | Sentence `"placement.coding_agent is literally 'unknown' — auto-detection from $CLAUDECODE / $CLAUDE_CODE_ENTRYPOINT env vars is deferred."` includes a deferred-feature mention. | KEEP | Per Reviewer BLOCKER 3, the same `auto-detection ... is deferred` clause also appears at `ARCHITECTURE.md` L38, `docs/spec/data-model.md` L31, and `cafleet/src/cafleet/broker.py` L17 (`FIXME(claude)` comment). "Deferred" is future-work language, not a `removal.md`-forbidden deprecation marker. To stay internally consistent, this design leaves all four occurrences in place rather than touch one of four. No edit. |

#### Source-config surface

| Path | Line | Issue | Classification | Action |
|---|---|---|---|---|
| `cafleet/pyproject.toml` | 58 | `"httpx.*"` is listed under `[tool.ty.analysis] allowed-unresolved-imports` even though no file in `cafleet/src/cafleet/` or `cafleet/tests/` imports `httpx`. Residue from the removed Auth0 client (0000015). | DELETE | Remove the `"httpx.*"` line from the list. |
| `cafleet/pyproject.toml` | 6–14 | Runtime `dependencies` audit per Q7 — current list is `fastapi`, `uvicorn[standard]`, `sqlalchemy>=2.0`, `alembic`, `click`, `pydantic`, `pydantic-settings`. Each is imported directly by the source tree. | KEEP | No dependency removal; the list is already minimal. Documented here for the audit trail. |

#### Test surface (Q4 ruling — sentinel-style tests are forbidden)

`.claude/rules/removal.md` resolves the apparent tension between "regression guard that the removed flag no longer parses" and "sentinel-style 'deprecated → error' tests are not": the tests below are sentinel-style ("if you import / pass the removed thing it errors"), not exercises of current behavior, so they are deleted.

| Path | Class / function | Issue | Classification | Action |
|---|---|---|---|---|
| `cafleet/tests/test_coding_agent.py` | `TestCodexConstantRemoved` (3 tests: `test_codex_constant_import_raises`, `test_coding_agents_registry_import_raises`, `test_get_coding_agent_helper_import_raises`) | Sentinel: assert `from cafleet.coding_agent import CODEX / CODING_AGENTS / get_coding_agent` raises `ImportError`. | DELETE | Remove the entire `TestCodexConstantRemoved` class. Keep its imports of `CLAUDE` and `CodingAgentConfig` already used elsewhere in the file. |
| `cafleet/tests/test_cli_member.py` | `test_no_bash_flag_no_longer_parses` (function in `TestPermissionMode`-adjacent area, ≈ line 374) | Sentinel: assert `cafleet member create --no-bash` exits with `No such option: --no-bash`. | DELETE | Remove the function. |
| `cafleet/tests/test_cli_member.py` | `test_allow_bash_flag_no_longer_parses` (function, ≈ line 405) | Sentinel: assert `--allow-bash` rejected. | DELETE | Remove the function. |
| `cafleet/tests/test_cli_member.py` | `TestCodingAgentFlagRemoved` (≈ line 437 onward) | Sentinel: assert `cafleet member create --coding-agent <anything>` rejected with `No such option: '--coding-agent'`. | DELETE | Remove the entire class. |

Tests that LOOK like sentinels but stay (positive current-behavior tests):

| Path | Class / function | Why it stays |
|---|---|---|
| `cafleet/tests/test_alembic_0002_upgrade.py` | (entire file) | Exercises the actual alembic 0002 migration; references to the old `api_keys` / `owner_sub` schema are migration content, not stale documentation. Per Q8, alembic history is the canonical record. |
| `cafleet/tests/test_coding_agent.py` | `TestPermissionArgs`, `TestPromptTemplates::test_claude_template_documents_dontask_mode` | Pin the canonical `--permission-mode dontAsk` argv shape and the `dontAsk` canary in the prompt template — current-behavior assertions, not removed-feature sentinels. |

#### Lockfile / dependency artifacts

| Path | Issue | Classification | Action |
|---|---|---|---|
| `admin/bun.lock` | Lines 8 / 35 / 37 / 39 still pin `@auth0/auth0-react`, `@auth0/auth0-auth-js`, `@auth0/auth0-spa-js` even though `admin/package.json` has no Auth0 dependency. Stale lockfile residue from 0000015. | UPDATE | Re-resolve the lockfile by running `bun install` in `admin/`. The resulting lockfile must contain no `@auth0/*` lines. |

#### Repository / hidden-file blind-spot sweep (Q9)

| Path / dir | Status | Notes |
|---|---|---|
| `scripts/` | DOES NOT EXIST | No directory at repo root; nothing to clean. Verified at draft time. |
| `.github/workflows/ci.yml` | CLEAN | Only `uv sync`, `ruff check`, `ty check`, `pytest tests/ -v`. No references to removed modules or flags. |
| `.pre-commit-config.yaml` | DOES NOT EXIST | Project does not use pre-commit. Nothing to clean. |
| `mise.toml` (root) | CLEAN | Three lines, no task definitions, only the monorepo `config_roots` declaration. |
| `cafleet/mise.toml` | CLEAN | Tasks reference live paths only (no removed modules). |
| `admin/mise.toml` | CLEAN | Three tasks (`lint`, `dev`, `build`) wrapping `bun`. |
| `.claude-plugin/plugin.json` | CLEAN | Skills list matches `skills/` directory contents (excluding `update-readme`, which is a project-only skill not exposed via the plugin). |
| `.gitignore` | CLEAN | No references to removed dirs or modules. |

#### 0000034 reconciliation

The body of `design-docs/0000034-member-bash-via-director/design-doc.md` still describes the round-5c spec (member spawn argv `--disallowedTools "Bash"`, `cafleet member create --no-bash` flag pair, `disallow_tools_args` field name) even though the R12 changelog entry (2026-04-29) records that the design was revised in-place to `--permission-mode dontAsk`. Two stale artifacts in 0000034 contradict the dontAsk reality:

1. The body sections §1 / §2 / §3 / §6 / §11 / etc. reference `--disallowedTools` and `--no-bash` directly. Per Q2, these are not rewritten — the R12 changelog already records the in-place revision and is authoritative.
2. The R11+R12 changelog entry (line 691) links to `design-docs/0000035-member-bash-whitelist/design-doc.md`. That directory was deleted in commit `d323007 chore: remove obsolete design doc` and the link is now broken.

The minimum reconciliation:

| Path | Line | Issue | Action |
|---|---|---|---|
| `design-docs/0000034-member-bash-via-director/design-doc.md` | 691 (R12 changelog entry, body text) | Broken hyperlink to deleted `0000035-member-bash-whitelist`. | UPDATE the link target to point to this design (`../0000035-repository-cleanup/design-doc.md`) and trim the trailing parenthetical "(Option D chosen; A/B/C in Future Work)" — the alternatives lived in the deleted doc and have no canonical home now. The replacement sentence reads: `"See [design-docs/0000035-repository-cleanup/design-doc.md](../0000035-repository-cleanup/design-doc.md) §"Specification → Decision: dontAsk is canonical (Q2 record)" for the canonical statement that --permission-mode dontAsk is the supported member-spawn discipline."` |

No other edits to 0000034. The body inconsistency is a known artifact of the in-place R12 revision and is documented in this design's §"Specification → Decision: dontAsk is canonical (Q2 record)" so no future reader has to navigate ambiguity.

#### Decision: dontAsk is canonical (Q2 record)

The canonical CAFleet member-spawn discipline is:

- **Argv**: `claude --permission-mode dontAsk [--name <member-name>] <prompt>`. The `dontAsk` token is set in `cafleet/src/cafleet/coding_agent.py` as `CLAUDE.permission_args = ("--permission-mode", "dontAsk")`.
- **Bash semantics**: The member's Bash tool is enabled. Permission prompts auto-resolve silently. Members run `cafleet` (and any other shell command) directly via the Bash tool — no `!` prefix, no Director routing for the default case.
- **Bash-via-Director** (`cafleet member send-input --bash <cmd>`) remains as an opt-in escape hatch for commands the Claude Code harness deny-list rejects (e.g. `git push`). It is no longer the default flow.

The earlier `--disallowedTools "Bash"` design (rounds 5c–10 of 0000034) was retired. No `--no-bash` / `--allow-bash` flag exists on `cafleet member create`. No `disallow_tools_args` field exists on `CodingAgentConfig`. Any future restoration is gated on a new design doc; this design ratifies the current behavior as the supported state.

### Acceptance grep set

After the cleanup, the following grep returns zero hits when run from the repository root with the listed exclusions:

```
grep -rln \
  --exclude-dir=design-docs \
  --exclude-dir=researches \
  --exclude-dir=.git \
  --exclude-dir=node_modules \
  --exclude-dir=.venv \
  --exclude-dir=__pycache__ \
  --exclude-dir=.pytest_cache \
  --exclude-dir=.ruff_cache \
  --exclude-dir=alembic \
  --exclude=test_alembic_*.py \
  --exclude=bun.lock \
  "auth0\|Auth0\|api_key\|api_key_hash\|api-key\|--api-key\|--url\|cafleet env\|MCP\|mcp_server\|codex\|CODEX\|get_coding_agent\|CODING_AGENTS\|--coding-agent\|--no-bash\|--allow-bash\|disallowedTools\|disallow_tools_args\|Goose\|Aider\|Cursor\|multi-runner" \
  .
```

Exclusions justified:

- `design-docs/` — historical record; the user explicitly directed that historical design docs are kept (overrides the strict letter of `removal.md`).
- `researches/` — gitignored research output directory (per `.claude/rules/git-workflow.md`); excluded so transient local research notes do not block acceptance. The directory does not exist at draft time but may be created locally during execution.
- `.git/`, `.venv/`, `node_modules/`, `__pycache__/`, `.pytest_cache/`, `.ruff_cache/` — build / VCS / cache artifacts, never source-of-truth.
- `alembic/` (versions) and `test_alembic_*.py` — migration code legitimately references the old `api_keys` schema as part of the migration itself.
- `bun.lock` — generated lockfile content; regenerated wholesale in Step 5 rather than hand-edited. After Step 5 runs, `admin/bun.lock` MUST contain zero `@auth0/*` lines (verified by an additional targeted grep, see §Verification).

New terms added in response to Reviewer feedback:

- `api_key_hash` — covers the BLOCKER 2 forensic-visibility sentences in `data-model.md` and `cli-options.md`.
- `--api-key`, `--url`, `cafleet env` — cover the BLOCKER 1 `## Removed CLI Options` subsection in `cli-options.md`.

`MCP` is allowed in `skills/design-doc-execute/roles/verifier.md` because those references describe **MCP tools used as a verification protocol** (Playwright MCP, HTTP MCP), not the removed CAFleet MCP-server feature. The grep above WILL hit them; the executor manually verifies on each match that the surrounding context describes a verification-tool usage and not the removed feature, then records the suppression in §Verification's note column.

`api-key` / `api_key` is allowed in `cafleet/src/cafleet/alembic/versions/0002_local_simplification.py` and `cafleet/tests/test_alembic_0002_upgrade.py` (covered by the exclusions above).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation cleanup (docs first per project rule)

- [x] Edit `ARCHITECTURE.md` line 32: delete only the leading clause `"No bearer tokens, no API keys, no Auth0. "`. The trailing sentence `"The session_id is a non-secret session identifier. Sessions are partitions for tidiness, not security boundaries."` remains verbatim. <!-- completed: 2026-04-29T15:00 -->
- [x] Edit `docs/spec/webui-api.md` line 13: replace `"No server-side session. No Auth0. The SPA manages the active session_id client-side via hash-based routing."` with `"No server-side session. The SPA manages the active session_id client-side via hash-based routing."`. (Drop only the `"No Auth0. "` clause.) <!-- completed: 2026-04-29T15:00 -->
- [x] Edit `docs/spec/cli-options.md`: delete the entire `## Removed CLI Options` subsection (lines 47–57 inclusive at draft time, including the trailing `"These removals keep secrets out of shell history..."` rationale sentence). The blank line between line 46 and the next section header (`## Agent ID (--agent-id)` currently at line 59) collapses to a single blank line in the result. <!-- completed: 2026-04-29T15:00 -->
- [x] Edit `docs/spec/cli-options.md` line 25 (the `--session-id <id>` row in the Global Options table): replace the parenthetical `"(opaque string; new sessions get a UUIDv4, migrated sessions reuse a 64-char hex value)"` with `"(opaque string; new sessions receive a UUIDv4)"`. The surrounding sentence (`"Also called the namespace identifier..."`) is unchanged. <!-- completed: 2026-04-29T15:00 -->
- [x] Edit `docs/spec/data-model.md` line 15 (the `sessions.session_id` row Notes column): replace `"Opaque string. New sessions receive a UUIDv4; migrated sessions reuse the original api_key_hash value (64-char hex)."` with `"Opaque string. New sessions receive a UUIDv4."`. <!-- completed: 2026-04-29T15:00 -->
- [x] Edit `design-docs/0000034-member-bash-via-director/design-doc.md` line 691 (the R11+R12 changelog body): replace the broken hyperlink to `design-docs/0000035-member-bash-whitelist/design-doc.md` with a hyperlink to `design-docs/0000035-repository-cleanup/design-doc.md` and rewrite the trailing parenthetical per §"0000034 reconciliation". No other edits to that file. <!-- completed: 2026-04-29T15:00 -->

### Step 2: Skill cleanup (skills second)

- [x] Edit `skills/cafleet/SKILL.md` line 448: delete the standalone sentence `"The CLI never inspects placement.coding_agent."`. Verify the surrounding paragraph still reads cleanly — adjust whitespace if a blank-line collision results. The line-558 `auto-detection ... is deferred` sentence is intentionally NOT touched (see §"Specification → Skills surface" for the consistency rationale). <!-- completed: 2026-04-29T15:05 -->

### Step 3: Source-config cleanup (source third — config-only, no source edits)

- [x] Edit `cafleet/pyproject.toml` `[tool.ty.analysis].allowed-unresolved-imports`: remove the `"httpx.*"` entry. The list shrinks from 6 entries to 5. <!-- completed: 2026-04-29T15:08 -->

### Step 4: Test cleanup (tests fourth — Q4 ruling on sentinel tests)

- [x] Edit `cafleet/tests/test_coding_agent.py`: delete the entire `class TestCodexConstantRemoved:` (3 test methods + class docstring). Keep all other classes. Verify imports at the top of the file still all have at least one consumer; remove any import that becomes unused. <!-- completed: 2026-04-29T15:15 -->
- [x] Edit `cafleet/tests/test_cli_member.py`: delete the `test_no_bash_flag_no_longer_parses` and `test_allow_bash_flag_no_longer_parses` functions. <!-- completed: 2026-04-29T15:15 -->
- [x] Edit `cafleet/tests/test_cli_member.py`: delete the entire `class TestCodingAgentFlagRemoved:` and any of its test methods. <!-- completed: 2026-04-29T15:15 -->
- [x] Run `mise //cafleet:test` and confirm the test count drops by at least 6 (3 from `TestCodexConstantRemoved`, 2 from the two stand-alone functions, ≥1 from `TestCodingAgentFlagRemoved`). Record the new total in the executor's PR description. <!-- completed: 2026-04-29T15:15 -->

### Step 5: Lockfile refresh (lockfile fifth — generated artifact)

- [x] In the `admin/` directory, run `bun install` to regenerate `admin/bun.lock` from the current `admin/package.json`. Do NOT hand-edit the lockfile. <!-- completed: 2026-04-29T15:25 -->
- [x] Hard gate: run `grep -nE '@auth0/' admin/bun.lock`. The expected output is zero matches. If any remain, run `bun why @auth0/auth0-react` (and the same for any other surviving `@auth0/*` package) inside `admin/` and report the dependency chain to the user. Do NOT proceed past Step 5 until the targeted grep returns zero hits. <!-- completed: 2026-04-29T15:25 -->

### Step 6: Verification (project rule order: lint → typecheck → test → grep)

- [x] Run `mise //cafleet:lint`. Confirm exit 0 and no new warnings introduced. <!-- completed: 2026-04-29T15:35 -->
- [x] Run `mise //cafleet:typecheck`. Confirm exit 0 and no new unresolved-import errors after the `httpx.*` removal (none expected, since no source file imports `httpx`). <!-- completed: 2026-04-29T15:35 -->
- [x] Run `mise //cafleet:test`. Confirm exit 0 and the test count delta from Step 4 is reflected. <!-- completed: 2026-04-29T15:35 -->
- [x] Run `mise //admin:lint`. Confirm exit 0. <!-- completed: 2026-04-29T15:35 -->
- [x] Run `mise //admin:build`. Confirm exit 0 and the resulting bundle is byte-for-byte unchanged from the pre-cleanup build (the Auth0 packages were already not imported by `admin/src/`). <!-- completed: 2026-04-29T15:35 -->
- [x] Execute the grep set from §"Acceptance grep set" exactly as documented. Verify the only hits are the documented suppressions (verifier.md MCP-tool references; alembic / test_alembic / bun.lock exclusions enforced by the flags). Record each suppression's reason inline in the PR description. <!-- completed: 2026-04-29T15:35 -->
- [x] Targeted grep `grep -nE '@auth0/' admin/bun.lock`: zero hits. <!-- completed: 2026-04-29T15:35 -->
- [x] Targeted grep `grep -nE 'httpx' cafleet/pyproject.toml`: zero hits. <!-- completed: 2026-04-29T15:35 -->
- [x] Targeted grep `grep -nE '## Removed CLI Options|api_key_hash|--api-key|cafleet env' docs/spec/cli-options.md docs/spec/data-model.md`: zero hits. <!-- completed: 2026-04-29T15:35 -->

### Step 7: Finalize

- [ ] Update this design doc: `**Status**: Approved` → `Complete` and refresh `**Last Updated**:` to the merge date. <!-- completed: -->
- [ ] Confirm header `**Progress**: N/N tasks complete` matches the actual `- [x]` count. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-29 | Initial draft. Director answers folded in: single PR, dontAsk decision recorded (Q2), `coding_agent.py` kept as-is (Q3), sentinel tests deleted (Q4), grep set used verbatim (Q5), `mise //admin:lint` + `mise //admin:build` added to verification (Q6), pyproject deps audit in scope (Q7 — only `httpx.*` `allowed-unresolved-imports` line found stale; runtime deps already minimal), alembic 0002 untouched (Q8), four blind-spot dirs swept (Q9 — `scripts/` doesn't exist, `.pre-commit-config.yaml` doesn't exist, `.github/workflows/ci.yml` clean, `mise.toml` files clean). One additional non-Q-list candidate surfaced during the survey: `admin/bun.lock` still pins `@auth0/*` packages even though `admin/package.json` no longer references them — added as Step 5. |
| 2026-04-29 | Reviewer-feedback revision (round 1). BLOCKER 1: `docs/spec/cli-options.md` lines 47–57 carry a `## Removed CLI Options` subsection enumerating `--url`, `--api-key`, env-var removals, and the `cafleet env` subcommand — direct violation of `removal.md`. Subsection deletion added as a new task in Step 1, grep set extended with `--api-key`, `--url`, `cafleet env`. BLOCKER 2: `docs/spec/data-model.md` line 15 and `cli-options.md` line 25 contain forensic-visibility sentences about migrated sessions reusing `api_key_hash` values, rewritten to describe only current behavior, `api_key_hash` added to the grep set. BLOCKER 3: dropped the `skills/cafleet/SKILL.md` line-558 edit because the same `auto-detection ... is deferred` clause appears at `ARCHITECTURE.md` L38, `data-model.md` L31, and `broker.py` L17 (`FIXME(claude)`). To stay internally consistent the design leaves all four occurrences in place — "deferred" is future-work language, not a forbidden deprecation marker. Step 2 task count drops from 2 to 1. P1: cross-ref at line 130 now matches the actual heading text including the `(Q2 record)` suffix, the link target inside the 0000034 reconciliation table also updated to point at `Decision: dontAsk is canonical (Q2 record)` rather than the reconciliation subsection. P2: `ARCHITECTURE.md` edit reformulated — instead of introducing `"not an authentication token"` (itself a soft what-it-isn't formulation), the executor deletes only the leading `"No bearer tokens, no API keys, no Auth0. "` clause and keeps the existing trailing sentence verbatim. P3: Step 5 task 2 hardened — the targeted `@auth0/` grep is now a hard gate, `bun why @auth0/auth0-react` is the diagnostic if any matches survive, Step 5 may not be marked complete with non-zero hits. P4: `researches/` added to the acceptance-grep exclusions list. Step 1 grew from 3 to 6 tasks, Step 2 shrank from 2 to 1, Step 6 gained one targeted-grep task for the new BLOCKER 1/2 terms. New implementation-only task count: 25. |
| 2026-04-29 | **User-approved.** Status moved Draft → Approved. Last Updated stamp 2026-04-29. `**Progress**: 0/25 tasks complete` retained (no implementation started). Implementation tasks spot-verified for actionability: every task carries a concrete file path (e.g. `cafleet/pyproject.toml`, `docs/spec/cli-options.md`, `cafleet/tests/test_coding_agent.py`), a concrete line number or symbol name (e.g. `class TestCodexConstantRemoved`, `[tool.ty.analysis].allowed-unresolved-imports`, `## Removed CLI Options` subsection), a verbatim before/after string for every text replacement, and a specific exit-code or grep-zero-hit assertion for every verification step. The two `bun install` and `mise //admin:build` tasks include byte-equality / dependency-chain diagnostics so an executor never has to invent its own pass criteria. Doc is ready for `/design-doc-execute` to begin implementation. |
