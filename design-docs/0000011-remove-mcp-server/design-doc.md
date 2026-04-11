# Remove MCP Server and SSE Endpoint

**Status**: Approved
**Progress**: 9/41 tasks complete
**Last Updated**: 2026-04-11

## Overview

Delete the `mcp-server/` package (`hikyaku-mcp` transparent proxy) and the broker's SSE notification stack (`/api/v1/subscribe`, `PubSubManager`, related tests, dependencies). After investigation the MCP proxy was the only production consumer of the SSE endpoint, so both layers are removed together as a single, clean cut.

## Success Criteria

- [ ] `mcp-server/` directory no longer exists in the repository
- [ ] No file outside `design-docs/`, `vendor/`, `.git/`, `node_modules/`, or `.venv/` references `mcp-server`, `hikyaku-mcp`, or `hikyaku_mcp`
- [ ] No file outside `design-docs/`, `vendor/`, `.git/`, `node_modules/`, or `.venv/` references `subscribe`, `PubSub`, `pubsub`, `sse_starlette`, `sse-starlette`, `httpx_sse`, or `httpx-sse` in production code paths
- [ ] `uv sync` succeeds against the slimmed workspace and `uv.lock` is regenerated
- [ ] `mise //:lint`, `mise //:format`, `mise //:typecheck` pass
- [ ] `mise //registry:test` and `mise //client:test` pass
- [ ] CI workflow has no `test-mcp-server` job
- [ ] README, ARCHITECTURE, and `.claude/` documentation no longer rationalize `--workers=1` via the SSE in-process Pub/Sub fan-out

---

## Background

The MCP proxy (`mcp-server/`) was introduced by design doc `0000005-streaming-subscribe` so that Claude Code's `poll` tool could return instantly from a locally SSE-buffered queue. The broker's SSE endpoint (`/api/v1/subscribe`) plus its in-process `PubSubManager` were built specifically to feed that proxy.

A pre-removal audit confirmed that **no other production code consumes `/api/v1/subscribe`**:

- The `hikyaku` CLI uses REST + JSON-RPC only (no SSE).
- The WebUI SPA uses `/ui/api/*` REST endpoints only (no SSE).
- The only client of `/api/v1/subscribe` is the `hikyaku-mcp` SSE background task.

Removing the MCP proxy therefore renders the entire SSE stack dead code. This design doc removes both in a single cycle so the codebase, dependency graph, and operational constraints all collapse together.

This document supersedes:

- All of `design-docs/0000005-streaming-subscribe/design-doc.md` — both the MCP proxy and the broker SSE endpoint that fed it are being removed.

Historical design docs `0000005`, `0000007`, `0000008`, and `0000009` remain unchanged as an immutable historical record.

---

## Specification

### Out of scope

- The broker's REST + JSON-RPC surfaces (`/api/v1/agents`, `/`, `/.well-known/agent-card.json`, `/ui/api/*`) — unchanged.
- The CLI (`client/`) — unchanged.
- The WebUI SPA (`admin/`) — unchanged.
- Any backwards-compatibility shim, deprecation warning, or migration guide for former MCP users — explicitly **not provided**. The project is pre-1.0; breakage is acceptable.

### Files to delete

| Path | Reason |
|---|---|
| `mcp-server/` (entire directory) | The MCP proxy package |
| `registry/src/hikyaku_registry/api/subscribe.py` | SSE endpoint router |
| `registry/src/hikyaku_registry/pubsub.py` | In-process Pub/Sub fan-out |
| `registry/tests/test_subscribe.py` | SSE endpoint unit tests |
| `registry/tests/test_pubsub.py` | PubSubManager unit tests |
| `registry/tests/test_e2e_subscribe.py` | SSE end-to-end integration tests |
| `docs/spec/streaming-subscribe.md` | SSE + MCP spec, fully obsolete |

### Files to edit (server code)

| Path | Change |
|---|---|
| `registry/src/hikyaku_registry/executor.py` | Drop the `pubsub` constructor parameter, the `self._pubsub` attribute, and both `self._pubsub.publish(f"inbox:{...}", task.id)` call sites in `_handle_unicast` and `_handle_broadcast`. The executor still saves Tasks via `TaskStore.save`; only the publish calls disappear. |
| `registry/src/hikyaku_registry/main.py` | Remove the imports of `subscribe_router`, `_get_pubsub`, `_get_subscribe_task_store`, and `PubSubManager`. Delete the `pubsub_manager = PubSubManager()` construction in `create_app()`, drop the `pubsub=pubsub_manager` argument to `BrokerExecutor(...)`, drop the `app.include_router(subscribe_router, prefix="/api/v1")` line, drop the two `dependency_overrides[_get_pubsub]` / `[_get_subscribe_task_store]` lines, and rewrite the `__main__` block comment so it no longer mentions PubSubManager (the `--workers=1` discussion is no longer load-bearing). |
| `registry/src/hikyaku_registry/api/__init__.py` | Already empty; no edit required. (Listed here only to confirm there is no `subscribe_router` re-export to remove.) |
| `registry/tests/conftest.py` | Drop the `pubsub_manager` mention in the module docstring. |
| `registry/tests/test_executor.py` | Delete `class TestExecutorPubSubIntegration` (lines ~1131 to end of class). The class only validates publish behavior that no longer exists. |

### Files to edit (workspace / build / CI)

| Path | Change |
|---|---|
| `pyproject.toml` (workspace root) | Remove `"mcp-server"` from `[tool.uv.workspace].members`. Remove the `hikyaku-mcp = { workspace = true }` entry from `[tool.uv.sources]`. Remove `"hikyaku-mcp"` from the `dev` dependency-group list. Drop `"sse_starlette.*"` and `"httpx_sse.*"` from `[tool.ty.analysis].allowed-unresolved-imports`. Keep `"httpx.*"` (used by `client/src/hikyaku_client/api.py`). |
| `registry/pyproject.toml` | Remove `"sse-starlette"` from `dependencies`. |
| `mise.toml` | Remove `"mcp-server"` from `[monorepo].config_roots`. |
| `.github/workflows/ci.yml` | Delete the entire `test-mcp-server` job. |

### Files to edit (documentation)

| Path | Change |
|---|---|
| `ARCHITECTURE.md` | Drop the entire `## Streaming Subscribe (SSE)` top-level section (including the MCP Server subsection). Drop the SSE Endpoint and `hikyaku-mcp` boxes from the architecture diagram. Drop the `mcp-server/` row from the "Monorepo Structure" section. Drop the four `mcp-server/` Component Layout rows (`server.py`, `sse_client.py`, `registry.py`, `config.py`) and the `pubsub.py` and `api/subscribe.py` rows. Drop the `pubsub_manager`, "in-process Pub/Sub", and `--workers=1` rationale from the "Storage Layer" / surrounding paragraphs. Drop the "Real-time inbox notification" row from the Responsibility Assignment table. Drop the "MCP proxy (all tools)" row from the same table. Drop the `MCP Server (`mcp-server/`)` column from the "CLI Option Sources" table, leaving only the `CLI (`client/`)` column. |
| `docs/spec/streaming-subscribe.md` | **Delete the file.** |
| `docs/spec/cli-options.md` | Drop the MCP server column from the Option Source Matrix; rename the section to refer to `hikyaku` only. Drop the "For the MCP server, also set the agent ID" code block and surrounding sentence. |
| `README.md` | Drop the "Real-time Inbox Notification" Features bullet. Drop the "MCP Server" Features bullet. Drop the entire "MCP Server (Claude Code Integration)" section. Drop the `hikyaku-mcp` ASCII box from the architecture diagram. Drop the `mcp-server/` row from the Project Structure tree. Drop "MCP Server: mcp + httpx + httpx-sse" from the Tech Stack list. Drop the `mise //mcp-server:test` line from Development. Remove the "broker must run with a single worker" Note paragraph (the constraint no longer has a load-bearing rationale). Drop any mention of `httpx-sse`, `sse-starlette`, `PubSubManager`, `/api/v1/subscribe`, or "in-process Pub/Sub". |
| `CLAUDE.md` | Drop the `mcp-server/` workspace bullet, the `hikyaku-mcp` package bullet, and "MCP Server: mcp + httpx + httpx-sse" from Tech Stack. |
| `.claude/CLAUDE.md` | Same as `CLAUDE.md`: drop the `mcp-server/` workspace bullet, the `hikyaku-mcp` package bullet, and "MCP Server: mcp + httpx + httpx-sse" from Tech Stack. |
| `.claude/rules/commands.md` | Drop `mise //mcp-server:test`, `mise //mcp-server:lint`, and `mise //mcp-server:dev` lines. |
| `.claude/skills/hikyaku/SKILL.md` | Drop the entire "MCP Server (Transparent Proxy)" section. Drop the "Receiving real-time inbox notifications" When-to-Use bullet. Drop any reference to the MCP server in the workflow narrative. |

### `--workers=1` decision

The single-worker constraint was documented because the in-process `PubSubManager` queue lived in one worker's memory and a publish in worker A could not reach a subscriber in worker B. With SSE removed, **the constraint has no remaining technical justification**.

**Decision**: Keep `mise //registry:dev` as-is. It currently runs `uv run src/hikyaku_registry/main.py`, which invokes the `__main__` block in `main.py` and calls `uvicorn.run(..., reload=True)` with no explicit `--workers` argument (uvicorn defaults to a single worker). No code change to the dev task is required.

**What changes**: Documentation. README, ARCHITECTURE, and the comment in `main.py`'s `__main__` block must no longer claim the single-worker mode is enforced "because of the SSE / PubSubManager fan-out". The `__main__` block's comment about `reload=True` should also be rewritten to drop the PubSubManager justification.

### Verification

After implementation, two grep checks must return zero hits in production code paths:

```bash
# MCP residue
grep -rn "mcp-server\|hikyaku-mcp\|hikyaku_mcp" \
  --exclude-dir=design-docs --exclude-dir=.git \
  --exclude-dir=node_modules --exclude-dir=.venv \
  --exclude-dir=vendor .

# SSE residue (Python sources)
grep -rn "subscribe\|PubSub\|pubsub\|sse_starlette\|sse-starlette\|httpx_sse\|httpx-sse" \
  --include='*.py' \
  --exclude-dir=design-docs --exclude-dir=.git \
  --exclude-dir=node_modules --exclude-dir=.venv \
  --exclude-dir=vendor .

# SSE residue (Markdown docs)
grep -rn "subscribe\|PubSub\|pubsub\|sse_starlette\|sse-starlette\|httpx_sse\|httpx-sse" \
  --include='*.md' \
  --exclude-dir=design-docs --exclude-dir=.git \
  --exclude-dir=node_modules --exclude-dir=.venv \
  --exclude-dir=vendor .
```

All three greps must return zero hits in production code. Historical design docs and vendor directories are exempt. The Markdown grep may surface incidental matches like `subscribe` inside Auth0 SDK terminology — inspect each remaining hit; only true SSE/PubSub residue is a failure.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
>
> **Ordering rule**: Per `.claude/rules/design-doc-numbering.md`, all documentation updates (Steps 1–4) must complete before any code, build, or CI edits (Steps 5+). Server-code edits live after package deletion to keep the doc-first sequence intact.

### Step 1: Update ARCHITECTURE.md

- [x] Delete the entire `## Streaming Subscribe (SSE)` top-level section, including the `### Server-side: SSE Endpoint`, `### In-Process Pub/Sub Integration`, and `### MCP Server (Transparent Proxy)` subsections <!-- completed: 2026-04-11T10:50 -->
- [x] Remove the SSE Endpoint box and the `hikyaku-mcp` proxy box from the architecture diagram; leave only the broker, A2A Server, SQLite, and tenant boxes <!-- completed: 2026-04-11T10:50 -->
- [x] Delete the `pubsub.py`, `api/subscribe.py`, and the four `mcp-server/` Component Layout rows (`server.py`, `sse_client.py`, `registry.py`, `config.py`) from the Component Layout table <!-- completed: 2026-04-11T10:50 -->
- [x] Delete the "Real-time inbox notification" and "MCP proxy (all tools)" rows from the Responsibility Assignment table <!-- completed: 2026-04-11T10:50 -->
- [x] Drop the `mcp-server/` workspace bullet from the "Monorepo Structure" section <!-- completed: 2026-04-11T10:50 -->
- [x] Remove every sentence that mentions "in-process Pub/Sub", "PubSubManager", "single-worker constraint", or `--workers=1` rationale tied to SSE; the broker's storage / dev-server discussion must read coherently without these references <!-- completed: 2026-04-11T10:50 -->
- [x] Drop the `MCP Server (` `mcp-server/` `)` column from the "CLI Option Sources" subsection table, leaving only the `CLI (` `client/` `)` column <!-- completed: 2026-04-11T10:55 -->

### Step 2: Update docs/spec/

- [x] Delete `docs/spec/streaming-subscribe.md` entirely <!-- completed: 2026-04-11T11:00 -->
- [x] Edit `docs/spec/cli-options.md`: drop the MCP Server column from the Option Source Matrix, drop the `HIKYAKU_AGENT_ID` "for the MCP server" subsection, and reword any remaining sentences so they refer to the `hikyaku` CLI only <!-- completed: 2026-04-11T11:00 -->

### Step 3: Update README.md

- [ ] Drop the "Real-time Inbox Notification" Features bullet and the "MCP Server" Features bullet <!-- completed: -->
- [ ] Delete the entire `## MCP Server (Claude Code Integration)` section, including the configuration JSON example <!-- completed: -->
- [ ] Remove the `hikyaku-mcp` proxy ASCII box from the architecture diagram <!-- completed: -->
- [ ] Drop the `mcp-server/` directory row from the Project Structure tree <!-- completed: -->
- [ ] Remove "MCP Server: mcp + httpx + httpx-sse" from the Tech Stack list <!-- completed: -->
- [ ] Remove the `mise //mcp-server:test` line from the Development section <!-- completed: -->
- [ ] Remove the "broker must run with a single worker (`--workers=1`)" Note paragraph and any other mention of `/api/v1/subscribe`, `PubSubManager`, `httpx-sse`, `sse-starlette`, or "in-process Pub/Sub" <!-- completed: -->

### Step 4: Update CLAUDE.md, .claude/, and skill docs

- [ ] Edit `CLAUDE.md`: drop the `mcp-server/` workspace bullet, the `hikyaku-mcp` package bullet, and "MCP Server: mcp + httpx + httpx-sse" from the Tech Stack list <!-- completed: -->
- [ ] Edit `.claude/CLAUDE.md`: same edits as `CLAUDE.md` <!-- completed: -->
- [ ] Edit `.claude/rules/commands.md`: drop the `mise //mcp-server:test`, `mise //mcp-server:lint`, and `mise //mcp-server:dev` lines from the Commands table <!-- completed: -->
- [ ] Edit `.claude/skills/hikyaku/SKILL.md`: delete the entire `## MCP Server (Transparent Proxy)` section, the "Receiving real-time inbox notifications" When-to-Use bullet, and any other MCP server narrative <!-- completed: -->

### Step 5: Delete the mcp-server package

- [ ] `rm -r mcp-server/` (removes `src/hikyaku_mcp/{__init__,server,sse_client,registry,config}.py`, `tests/{__init__,test_server,test_sse_client,test_registry}.py`, `pyproject.toml`, and `mise.toml`) <!-- completed: -->

### Step 6: Delete SSE server code

- [ ] Delete `registry/src/hikyaku_registry/api/subscribe.py` <!-- completed: -->
- [ ] Delete `registry/src/hikyaku_registry/pubsub.py` <!-- completed: -->
- [ ] Delete `registry/tests/test_subscribe.py` <!-- completed: -->
- [ ] Delete `registry/tests/test_pubsub.py` <!-- completed: -->
- [ ] Delete `registry/tests/test_e2e_subscribe.py` <!-- completed: -->

### Step 7: Edit registry server code

- [ ] Edit `registry/src/hikyaku_registry/executor.py`: drop the `pubsub` parameter from `BrokerExecutor.__init__`, drop the `self._pubsub = pubsub` assignment, and delete the two `if self._pubsub is not None: await self._pubsub.publish(f"inbox:{...}", delivery_task.id)` blocks in `_handle_unicast` and `_handle_broadcast`. The executor must still call `self._task_store.save(...)` and `event_queue.enqueue_event(...)` exactly as before <!-- completed: -->
- [ ] Edit `registry/src/hikyaku_registry/main.py`: remove the imports of `subscribe_router`, `_get_pubsub`, `_get_task_store as _get_subscribe_task_store`, and `PubSubManager`. Delete the `pubsub_manager = PubSubManager()` line and the `pubsub=pubsub_manager` argument to `BrokerExecutor(...)`. Delete the `app.include_router(subscribe_router, prefix="/api/v1")` line. Delete the `app.dependency_overrides[_get_pubsub]` and `app.dependency_overrides[_get_subscribe_task_store]` lines. Rewrite the `__main__` block's comment so it no longer mentions `PubSubManager` or the SSE fan-out — keep it as a short note about `reload=True` being a developer convenience only <!-- completed: -->
- [ ] Edit `registry/tests/conftest.py`: remove the `pubsub_manager` mention in the module docstring (Step 7 reference no longer exists) <!-- completed: -->
- [ ] Edit `registry/tests/test_executor.py`: delete the `TestExecutorPubSubIntegration` class (every test method that exercises the `pubsub` parameter, including `test_init_accepts_pubsub_parameter`, the publish-channel tests, and the no-pubsub backward-compat tests) <!-- completed: -->

### Step 8: Edit registry/pyproject.toml

- [ ] Remove `"sse-starlette"` from the `dependencies` list in `registry/pyproject.toml` <!-- completed: -->

### Step 9: Edit workspace pyproject.toml

- [ ] Remove `"mcp-server"` from `[tool.uv.workspace].members` so the list contains only `["registry", "client"]` <!-- completed: -->
- [ ] Remove the `hikyaku-mcp = { workspace = true }` line from `[tool.uv.sources]` and the `"hikyaku-mcp"` entry from the `dev` dependency group <!-- completed: -->
- [ ] Remove `"sse_starlette.*"` and `"httpx_sse.*"` from `[tool.ty.analysis].allowed-unresolved-imports` (keep `"httpx.*"`, which is still used by `hikyaku-client`) <!-- completed: -->

### Step 10: Edit mise.toml

- [ ] Remove `"mcp-server"` from `[monorepo].config_roots` so the list contains only `["registry", "client", "admin"]` <!-- completed: -->

### Step 11: Edit .github/workflows/ci.yml

- [ ] Delete the entire `test-mcp-server` job (all five steps under it) <!-- completed: -->

### Step 12: Regenerate uv.lock

- [ ] Run `uv sync` from the project root to refresh `uv.lock` against the slimmed workspace <!-- completed: -->

### Step 13: Verify

- [ ] Run `mise //:lint`, `mise //:format`, `mise //:typecheck`; all must pass <!-- completed: -->
- [ ] Run `mise //registry:test` and `mise //client:test`; all must pass <!-- completed: -->
- [ ] Run the MCP residue grep (`grep -rn "mcp-server\|hikyaku-mcp\|hikyaku_mcp" --exclude-dir=design-docs --exclude-dir=.git --exclude-dir=node_modules --exclude-dir=.venv --exclude-dir=vendor .`) and confirm zero hits <!-- completed: -->
- [ ] Run the SSE residue greps scoped to `--include='*.py'` and `--include='*.md'` separately (see Specification → Verification) and inspect every remaining hit; only incidental matches in third-party SDK terminology (e.g., Auth0 in `admin/`) are acceptable <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-11 | Initial draft. Scope expanded mid-clarification: SSE endpoint and `PubSubManager` are removed alongside `mcp-server/` because the MCP proxy was the only production consumer. |
