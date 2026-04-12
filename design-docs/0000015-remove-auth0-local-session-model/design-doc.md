# Remove Auth0: Local-Only Session Model

**Status**: Approved
**Progress**: 11/80 tasks complete
**Last Updated**: 2026-04-12

## Overview

Collapse Hikyaku's multi-tenant access-control model into a single local-only namespace. Auth0, tenants, and API keys are removed end-to-end; a non-secret `session_id` becomes the sole namespace key, minted via a new `hikyaku-registry session create` administrative command that writes directly to SQLite. New sessions receive a UUIDv4; migrated sessions reuse the existing `api_key_hash` value verbatim so no data rewrite is required. The broker runs without authentication on the assumption that only the local developer can reach it.

## Success Criteria

- [ ] `@auth0/auth0-react`, `PyJWT` JWKS flow, `Auth0Verifier`, `verify_auth0_user`, `get_user_id`, and every `VITE_AUTH0_*` / `AUTH0_*` env var are fully deleted from the codebase
- [ ] `api_keys` table, `owner_sub` column, `ApiKey` model, `is_key_owner`, `create_api_key`, `list_api_keys`, `revoke_api_key`, and `is_api_key_active` are fully deleted
- [ ] New `sessions` table exists with schema `(session_id TEXT PK, label TEXT NULL, created_at TEXT NOT NULL)`, declared in `db/models.py` and created by Alembic migration `0002_local_simplification`
- [ ] `agents.tenant_id` column is renamed to `session_id` in the same migration; FK retargets `sessions.session_id`; index `idx_agents_tenant_status` is renamed to `idx_agents_session_status`
- [ ] `hikyaku-registry session create [--label <text>]` mints a session by writing a row directly to SQLite (no HTTP) and prints the `session_id` in both human and `--json` modes
- [ ] `hikyaku-registry session list | show <id> | delete <id>` are implemented alongside `create`
- [ ] `hikyaku` client CLI reads `HIKYAKU_SESSION_ID` (not `HIKYAKU_API_KEY`) and forwards it to the broker via `X-Session-Id` header
- [ ] `HIKYAKU_URL` falls back to `http://127.0.0.1:8000` in the `hikyaku` client CLI when unset
- [ ] `client/` workspace has **no** SQLAlchemy dependency (admin ops live in `hikyaku-registry` only)
- [ ] Broker `POST /` JSON-RPC, `POST/GET/DELETE /api/v1/agents*`, and every `/ui/api/*` endpoint use `X-Session-Id` and do not require a bearer token
- [ ] Cross-session sends are rejected with JSON-RPC error `-32003` (`SESSION_MISMATCH`); cross-session reads return HTTP 404
- [ ] `GET /ui/api/auth/config`, `POST /ui/api/keys`, `GET /ui/api/keys`, `DELETE /ui/api/keys/{tenant_id}` are deleted
- [ ] WebUI has no login screen; first-load lands on a session picker at `/ui/#/sessions`; selecting a session navigates to `/ui/#/sessions/<uuid>/agents`
- [ ] `GET /ui/api/sessions` exists and returns all rows from the `sessions` table
- [ ] `registry/tests/test_auth0.py` is deleted
- [ ] `registry/tests/test_key_endpoints.py` is deleted
- [ ] Documentation updates ship **before** any code changes: `README.md`, `ARCHITECTURE.md`, `docs/spec/data-model.md`, `docs/spec/registry-api.md`, `docs/spec/webui-api.md`, `docs/spec/cli-options.md`, `docs/spec/a2a-operations.md`, `.claude/skills/hikyaku/SKILL.md`
- [ ] `.claude/settings.json` replaces `Bash(printenv HIKYAKU_URL HIKYAKU_API_KEY)` with `Bash(printenv HIKYAKU_URL HIKYAKU_SESSION_ID)` and adds `Bash(hikyaku-registry session *)`
- [ ] `admin/mise.toml` `VITE_AUTH0_REDIRECT_URI` entry is removed
- [ ] `mise //:lint`, `mise //:format`, `mise //:typecheck`, `mise //registry:test`, `mise //client:test`, `mise //admin:lint`, and `mise //admin:build` all pass

---

## Background

Hikyaku was originally designed as a multi-tenant A2A broker with Auth0 OIDC for webUI login and a shared API key for agent-to-broker authentication. In practice it is used by a single developer running every component on one machine to coordinate several Claude Code agents across tmux panes (see `.claude/skills/hikyaku/SKILL.md`, Multi-Session Coordination). The multi-tenant machinery has no production consumer and its two load-bearing concepts — Auth0 and tenant-scoped API keys — are pure overhead:

| Pain | Location |
|---|---|
| Auth0 tenant + client_id + audience must be configured before the webUI works at all | `registry/src/hikyaku_registry/config.py:20-22`, `admin/mise.toml:6`, `admin/src/App.tsx:108-119` |
| PyJWT JWKS roundtrip on every webUI request | `registry/src/hikyaku_registry/auth.py:77-120` |
| Tenant is derived from `sha256(api_key)`, which means the API key is effectively used twice — as an opaque credential AND as the routing namespace | `registry/src/hikyaku_registry/auth.py:28-53`, `registry_store.py:167-174` |
| `api_keys.owner_sub` exists solely to tie an API key back to an Auth0 `sub` claim so the webUI can show "my keys" | `registry/src/hikyaku_registry/db/models.py:17-26`, `webui_api.py:213-242` |
| Key management lives in the webUI (`POST/GET/DELETE /ui/api/keys`), which requires the user to first log in via Auth0 just to mint the first key | `webui_api.py:213-242` |
| SKILL.md's tmux spawn recipe already treats the API key as a plain env var passed between panes — the "secret" is ceremonial | `.claude/skills/hikyaku/SKILL.md:198-228` |

The user has reframed the project: `session_id` is just a non-secret namespace, the broker trusts its local network, key listing is fine because there is no secret to protect, and session creation belongs with the existing `hikyaku-registry db init` administrative CLI rather than the runtime `hikyaku` client.

### Design docs this supersedes

This document supersedes `design-docs/0000002-access-control/design-doc.md` and `design-docs/0000007-api-key-specification/design-doc.md`. Per user direction, those files are NOT edited in place — the supersede relationship lives only here, to minimize churn on completed historical docs.

### Related in-flight work

`design-docs/0000014-hikyaku-member-lifecycle/design-doc.md` is **Approved** (0/44 tasks, frozen). It adds an `agent_placements` table and a `hikyaku member` CLI group, and currently assumes `agents.tenant_id` and `HIKYAKU_API_KEY`. Both this document and 0000014 claim the `Bash(printenv HIKYAKU_URL HIKYAKU_API_KEY)` settings.json cleanup in their Success Criteria; **this document owns that edit** (see Coordination Note §8). When 0000014 unfreezes, it rebases its criteria onto the already-renamed `HIKYAKU_SESSION_ID` entry.

`design-docs/0000013-admin-discord-style-timeline/design-doc.md` is Approved but 0/38 tasks and assumes Auth0 + tenant-scoped keys. Its Status will be flipped to `Blocked on 0000015` in a single-line edit; the body is not touched.

---

## Specification

### 1. Data Model

#### 1.1 New table: `sessions`

```python
class Session(Base):
    __tablename__ = "sessions"

    session_id: Mapped[str] = mapped_column(String, primary_key=True)  # opaque string
    label: Mapped[str | None] = mapped_column(String, nullable=True)
    created_at: Mapped[str] = mapped_column(String, nullable=False)    # ISO 8601 UTC
```

Constraints:
- `session_id` is an opaque server-chosen string. New sessions created via `hikyaku-registry session create` receive a canonical UUIDv4 (`550e8400-e29b-41d4-a716-446655440000`). Migrated sessions (from the 0002 upgrade) reuse the original `api_key_hash` value verbatim — a 64-char hex string — so no data rewrite is required. Application code must not assume UUID format; it treats `session_id` as an arbitrary `TEXT` primary key.
- No `status` column. No soft-revoke. Deletion is the only removal path and is rejected while agents still reference the session (FK `ondelete="RESTRICT"`).
- `label` is optional free-form text for human bookkeeping (e.g. `"PR-42 review"`). It is displayed in the webUI picker and in `session list` output but never used for routing.

#### 1.2 Dropped table: `api_keys`

Removed entirely. No replacement column for `owner_sub` — there is no ownership concept in the new model.

#### 1.3 Renamed column: `agents.tenant_id` → `agents.session_id`

```python
class Agent(Base):
    __tablename__ = "agents"

    agent_id: Mapped[str] = mapped_column(String, primary_key=True)
    session_id: Mapped[str] = mapped_column(          # was tenant_id
        String,
        ForeignKey("sessions.session_id", ondelete="RESTRICT"),
        nullable=False,
    )
    name: Mapped[str] = mapped_column(String, nullable=False)
    description: Mapped[str] = mapped_column(String, nullable=False)
    status: Mapped[str] = mapped_column(String, nullable=False)
    registered_at: Mapped[str] = mapped_column(String, nullable=False)
    deregistered_at: Mapped[str | None] = mapped_column(String, nullable=True)
    agent_card_json: Mapped[str] = mapped_column(String, nullable=False)

    __table_args__ = (Index("idx_agents_session_status", "session_id", "status"),)
```

Index `idx_agents_tenant_status` is renamed to `idx_agents_session_status`.

#### 1.4 Unchanged table: `tasks`

`tasks` routes messages by `context_id` (→ `agents.agent_id`). It never stored `tenant_id` directly, so the rename is transparent.

### 2. Alembic Migration `0002_local_simplification`

Single migration covering the entire schema change. No downgrade path.

File: `registry/src/hikyaku_registry/alembic/versions/0002_local_simplification.py`

```python
"""local simplification: drop api_keys+owner_sub, add sessions, rename agents.tenant_id to session_id

Revision ID: 0002_local_simplification
Revises: 0001
Create Date: 2026-04-12 ...
"""

from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa


revision: str = "0002_local_simplification"
down_revision: Union[str, None] = "0001"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # 1. Create sessions table.
    op.create_table(
        "sessions",
        sa.Column("session_id", sa.String(), nullable=False),
        sa.Column("label", sa.String(), nullable=True),
        sa.Column("created_at", sa.String(), nullable=False),
        sa.PrimaryKeyConstraint("session_id"),
    )

    # 2. Seed one session per api_keys row (active AND revoked), using
    #    the api_key_hash as the session_id. This preserves existing
    #    agents.tenant_id values byte-for-byte so step 3 becomes a pure rename.
    #    Including revoked keys prevents FK violations for agents whose
    #    tenant_id references a revoked key row.
    op.execute(
        """
        INSERT INTO sessions (session_id, label, created_at)
        SELECT api_key_hash, 'legacy-' || key_prefix, created_at
        FROM api_keys
        """
    )

    # 3. Rename agents.tenant_id -> session_id, add new FK into sessions,
    #    create new index. SQLite batch mode rebuilds the table from
    #    scratch, so no explicit drop_constraint / drop_index is needed —
    #    the old FK and index are discarded when the table is recreated.
    with op.batch_alter_table("agents", schema=None) as batch_op:
        batch_op.alter_column("tenant_id", new_column_name="session_id")
        batch_op.create_foreign_key(
            "fk_agents_session",
            "sessions",
            ["session_id"],
            ["session_id"],
            ondelete="RESTRICT",
        )
        batch_op.create_index(
            "idx_agents_session_status", ["session_id", "status"], unique=False
        )

    # 4. Drop api_keys entirely.
    with op.batch_alter_table("api_keys", schema=None) as batch_op:
        batch_op.drop_index("idx_api_keys_owner")
    op.drop_table("api_keys")


def downgrade() -> None:
    raise NotImplementedError(
        "0002_local_simplification is a one-way migration. "
        "Auth0 re-introduction is out of scope; restore from a backup instead."
    )
```

Notes:
- SQLite does not store FK constraint names. Alembic's `batch_alter_table` rewrites the full table from scratch, so no explicit `drop_constraint` or `drop_index` calls are needed — the old FK and index are discarded during the rebuild and the new FK + index are created by the `create_foreign_key` / `create_index` calls.
- The seeding step (2) seeds ALL `api_keys` rows (active and revoked) to prevent FK violations during step 3. Agents whose `tenant_id` references a revoked key would otherwise be orphaned when the FK retargets `sessions`. Fresh databases have zero `api_keys` rows and the INSERT is a no-op.
- Migrated sessions have `session_id` values that are 64-char SHA-256 hex strings (inherited from `api_key_hash`), not UUIDs. New sessions created via `hikyaku-registry session create` will be UUIDv4. Application code treats `session_id` as an opaque `TEXT` key and must not validate UUID format.
- `downgrade()` raises immediately — Alembic treats this as a one-way revision. This is called out in the docstring so future operators do not assume rollback is possible.

### 3. Broker HTTP Surface

| Path | Method | Before | After |
|---|---|---|---|
| `/api/v1/agents` | POST | Bearer API key → `tenant_id` from hash; body `{name, description, skills}` | **No bearer.** Body `{session_id, name, description, skills}`. Server 404s if `session_id` does not exist in `sessions` table. |
| `/api/v1/agents` | GET | Tenant from bearer | Query param `?session_id=<uuid>` (required). Lists agents in that session. |
| `/api/v1/agents/{id}` | GET | Bearer + tenant-scoped 404 | **No bearer.** Header `X-Session-Id: <uuid>` (required). 404 if agent missing OR agent.session_id ≠ header. |
| `/api/v1/agents/{id}` | DELETE | Bearer + self-check | **No bearer.** Header `X-Agent-Id: {id}` (self-identification). Returns 204 on success. Session is not re-verified. |
| `/` (JSON-RPC) | POST | Bearer + `X-Agent-Id` | **No bearer.** Header `X-Agent-Id: <caller>`. Broker resolves caller's session via `SELECT session_id FROM agents WHERE agent_id = ?`. Cross-session sends rejected with JSON-RPC error `-32003` "session mismatch" (raised as `SessionMismatchError` in `executor.py`, caught specifically in `main.py`'s `jsonrpc_endpoint`). |
| `/ui/api/sessions` | GET | — | **New.** Returns `[{session_id, label, created_at, agent_count}, ...]`. No auth, no header required. |
| `/ui/api/agents` | GET | Auth0 JWT + `X-Tenant-Id` | **No Auth0.** Query param `?session_id=<uuid>`. `X-Tenant-Id` header removed. |
| `/ui/api/agents/{id}/inbox` | GET | Auth0 JWT + `X-Tenant-Id` | **No Auth0.** Header `X-Session-Id: <uuid>` replaces both `Authorization` and `X-Tenant-Id`. |
| `/ui/api/agents/{id}/sent` | GET | Auth0 JWT + `X-Tenant-Id` | **No Auth0.** Header `X-Session-Id: <uuid>` replaces both `Authorization` and `X-Tenant-Id`. |
| `/ui/api/messages/send` | POST | Auth0 JWT + `X-Tenant-Id` | **No Auth0.** Header `X-Session-Id: <uuid>` replaces both `Authorization` and `X-Tenant-Id`. |
| `/ui/api/auth/config` | GET | Auth0 settings | **Deleted.** |
| `/ui/api/keys` | POST/GET | Auth0-scoped key CRUD | **Deleted.** |
| `/ui/api/keys/{id}` | DELETE | Auth0-scoped revoke | **Deleted.** |

#### 3.1 Error taxonomy

| Scenario | HTTP status | JSON body |
|---|---|---|
| `session_id` missing from required header/body/query | 400 | `{"error":{"code":"SESSION_REQUIRED","message":"..."}}` |
| `session_id` does not exist in `sessions` table | 404 | `{"error":{"code":"SESSION_NOT_FOUND","message":"..."}}` |
| `X-Agent-Id` header missing on JSON-RPC or REST endpoints that require it | 400 | `{"error":{"code":"AGENT_ID_REQUIRED","message":"..."}}` |
| `X-Agent-Id` present but agent does not exist | 404 | `{"error":{"code":"AGENT_NOT_FOUND","message":"..."}}` |
| Unicast target in a different session than caller | 404 | `{"error":{"code":"AGENT_NOT_FOUND","message":"..."}}` (indistinguishable from "missing" by design) |
| JSON-RPC send from agent A to agent B across sessions | JSON-RPC error | `{"code":-32003,"message":"Session mismatch"}` |

The code `-32003` is collision-free: `main.py:328,332` uses `-32601` (method not found) and `-32000` (generic server error); `docs/spec/a2a-operations.md` reserves `-32001` for `TaskNotFoundError` and `-32002` for `TaskNotCancelableError`. `-32003` is inside the JSON-RPC 2.0 reserved server-error range (`-32099` to `-32000`) and unassigned. No 401 responses remain — the broker no longer performs authentication, so the 401 HTTP status is removed from the entire surface.

Rationale: The broker no longer performs authentication. It performs **namespace routing** — `session_id` is a filter, not a credential. Cross-session 404 rather than 403 is deliberate: a caller who knows a target's session_id can always observe it (there is no secret to protect), but the default responses keep sessions structurally isolated so accidental cross-session traffic produces the same shape of error as "the agent does not exist at all."

#### 3.2 Bind address

`broker_host` default remains `0.0.0.0`. **No** code-level guard rail, startup check, or `--force-public` flag. Instead, a prominent warning is added to `README.md` and `docs/spec/` documenting that Hikyaku is a local-only tool and binding to a public interface is a user-owned decision. Any user who binds `0.0.0.0` on a shared network is accepting that every listener can see and act within every session.

The apparent asymmetry — server binds `0.0.0.0` while the client defaults to `127.0.0.1` — is deliberate per user direction. `0.0.0.0` is a convenience for Docker and VM setups where the developer tunnels from a different interface; the client defaults to loopback because the overwhelmingly common case is same-machine usage. The risk is documented, not code-gated.

### 4. CLI

#### 4.1 `hikyaku-registry session` — new subcommand group

Lives in `registry/src/hikyaku_registry/cli.py` alongside the existing `db init` command. Opens the SQLite file directly using a sync `create_engine(sync_url)` call — the same pattern `db init` already uses — so the broker server does not need to be running. The session commands do NOT use `RegistryStore` (which is async and designed for the runtime server); they run direct SQLAlchemy Core `INSERT`/`SELECT`/`DELETE` statements via a sync `Session`.

```
hikyaku-registry session create [--label TEXT]
hikyaku-registry session list
hikyaku-registry session show <session_id>
hikyaku-registry session delete <session_id>
```

| Command | Behavior |
|---|---|
| `session create` | `uuid.uuid4()` → insert row → print UUID on stdout. With `--label`, stores label. Idempotency: none — each call mints a fresh UUID. |
| `session list` | Prints one row per session: `session_id`, `label`, `created_at`, `agent_count` (computed via `LEFT JOIN agents WHERE status='active'`). `--json` flag for machine-parseable output. |
| `session show <id>` | Prints the single session's row if present; exits non-zero with `ERROR: session not found` otherwise. |
| `session delete <id>` | Attempts hard delete. Fails with `ERROR: session <id> has N active agents` (FK `RESTRICT` violation → converted to a friendly message) if agents still reference it. |

No `--json` toggle on the click group — add per-command where needed (`create`, `list`, `show`), mirroring how `hikyaku-client` does it today. `delete` is text-only: prints `Deleted session <id>` on success, exits 0 with no stdout.

The click group is a sibling of `db`, not a child:

```python
@main.group()
def db() -> None:
    """Database schema management commands."""

@main.group()
def session() -> None:
    """Session namespace management commands."""
```

`hikyaku-registry db init` remains schema-only. It does NOT auto-create a default session — the user creates sessions explicitly.

#### 4.2 `hikyaku` — runtime CLI rename

`client/src/hikyaku_client/cli.py` and `client/src/hikyaku_client/api.py` change in five ways:

1. **Env var rename**: Every reference to `HIKYAKU_API_KEY` → `HIKYAKU_SESSION_ID`. Error messages updated. The CLI exits with `Error: HIKYAKU_SESSION_ID environment variable is required. Create a session with 'hikyaku-registry session create'.` when unset.

2. **HTTP header rename**: `api.py` sends `X-Session-Id: <value>` instead of `Authorization: Bearer <value>`. The `X-Agent-Id` header continues to identify the caller.

3. **URL fallback**: `HIKYAKU_URL` defaults to `http://127.0.0.1:8000` when unset (previously `http://localhost:8000` — tighten to loopback to match the local-only stance).

4. **POST body and query param updates**: `api.register_agent` adds `session_id` to the POST body (matching the new `POST /api/v1/agents` contract). `api.list_agents` passes `?session_id=<value>` as a query parameter on `GET /api/v1/agents` (replacing the former bearer-derived tenant scoping).

5. **NO new dependencies**: `client/` remains HTTP-only. SQLAlchemy/aiosqlite stay in `registry/` only. `hikyaku session create` does NOT exist on the runtime CLI.

**Behavior change for `hikyaku register`**: `register` is the one command whose behavior tightens rather than just renaming. In the current code it is the only runtime command that does NOT call `_require_api_key` — historically it minted the key itself. Under the new model it gains a `_require_session_id` entry check and sends `session_id` in both the POST body and the `X-Session-Id` header, matching every other command's code path.

#### 4.3 CLI surface reference (unchanged commands)

The following commands remain in `hikyaku` and their semantics are identical, only the auth header changes:

- `hikyaku register`
- `hikyaku send`
- `hikyaku broadcast`
- `hikyaku poll`
- `hikyaku ack`
- `hikyaku cancel`
- `hikyaku get-task`
- `hikyaku agents`
- `hikyaku deregister`

All of them read `HIKYAKU_SESSION_ID` from the environment, set `X-Session-Id` on every request, and forward `X-Agent-Id` as before.

### 5. WebUI

#### 5.1 Deletions

| File | Action |
|---|---|
| `admin/src/components/LoginPage.tsx` | Delete |
| `admin/src/components/KeyManagement.tsx` | Delete |
| `admin/src/App.tsx` | Remove `Auth0Provider`, `useAuth0`, `getAuthConfig` call, `tokenReady` state, `isAuthenticated` gating, `LoginPage` branch, `KeyManagement` branch. Replace with `SessionPicker` → `Dashboard` router. |
| `admin/src/api.ts` | Remove `setGetAccessToken`, `getAccessToken` closure, `getAuthConfig`, `createKey`, `listKeys`, `revokeKey`. Remove both the `Authorization: Bearer` header (line 34) **and** the `X-Tenant-Id` header (line 38) from `request<T>`; replace with a single `X-Session-Id` header. Rename `setTenantId`/`getTenantId` closures (lines 10, 16, 20) to `setSessionId`/`getSessionId`. Add `listSessions()`. |
| `admin/package.json` | Remove `@auth0/auth0-react` dependency; run `bun install` to update `bun.lock`. |
| `admin/mise.toml` | Delete the `VITE_AUTH0_REDIRECT_URI` env entry (line 6). |
| `admin/.env*` (any Auth0 lines) | Delete |

#### 5.2 Additions

**New component**: `admin/src/components/SessionPicker.tsx`

```tsx
// Sketch — full implementation in Step 9.
export default function SessionPicker({
  onSelect,
}: {
  onSelect: (sessionId: string) => void;
}) {
  const [sessions, setSessions] = useState<SessionListItem[]>([]);
  useEffect(() => {
    listSessions().then(setSessions);
  }, []);

  if (sessions.length === 0) {
    return (
      <div>
        <p>No sessions found.</p>
        <p>Run <code>hikyaku-registry session create --label "my session"</code> to create one.</p>
      </div>
    );
  }

  return (
    <ul>
      {sessions.map((s) => (
        <li key={s.session_id}>
          <button onClick={() => onSelect(s.session_id)}>
            {s.label ?? s.session_id} — {s.agent_count} agents
          </button>
        </li>
      ))}
    </ul>
  );
}
```

**New API client function** in `admin/src/api.ts`:

```ts
export async function listSessions(): Promise<SessionListItem[]> {
  const resp = await fetch("/ui/api/sessions");
  if (!resp.ok) throw new Error("Failed to load sessions");
  return resp.json();
}
```

**New type** in `admin/src/types.ts`:

```ts
export interface SessionListItem {
  session_id: string;
  label: string | null;
  created_at: string;
  agent_count: number;
}
```

#### 5.3 Routing

The WebUI uses hash-based URL routing to embed the active session:

| URL | View |
|---|---|
| `/ui/` | Redirect to `/ui/#/sessions` |
| `/ui/#/sessions` | `<SessionPicker>` |
| `/ui/#/sessions/<uuid>/agents` | `<Dashboard sessionId={...} />` |
| `/ui/#/sessions/<uuid>/agents/<agent_id>/inbox` | Agent inbox tab |
| `/ui/#/sessions/<uuid>/agents/<agent_id>/sent` | Agent sent tab |

`App.tsx` parses `window.location.hash` on mount and on `hashchange`. No external router library (`react-router`) is added — the existing codebase has no router today, and a two-screen hash parser keeps the SPA bundle small.

`api.ts`'s `setTenantId` function is renamed `setSessionId`. Every outgoing `/ui/api/*` request includes the `X-Session-Id: <active>` header.

### 6. tmux Multi-Session Coordination

The flow in `.claude/skills/hikyaku/SKILL.md:198-228` is mechanically equivalent — only the env var name changes. The new recipe:

```bash
# One-time per team (run by the Director):
hikyaku-registry session create --label "PR-42 review"
# → prints: 550e8400-e29b-41d4-a716-446655440000
export HIKYAKU_SESSION_ID=550e8400-e29b-41d4-a716-446655440000

# Director registers itself:
hikyaku register --name "Director" --description "..."

# For each member:
printenv HIKYAKU_URL HIKYAKU_SESSION_ID
tmux split-window \
  -e "HIKYAKU_URL=http://127.0.0.1:8000" \
  -e "HIKYAKU_SESSION_ID=550e8400-e29b-41d4-a716-446655440000" \
  claude "Load Skill(hikyaku), register as Claude-B, send a ping, poll, ack, deregister."
tmux select-layout main-vertical
```

The `printenv` line in `.claude/settings.json` allow list updates from `Bash(printenv HIKYAKU_URL HIKYAKU_API_KEY)` to `Bash(printenv HIKYAKU_URL HIKYAKU_SESSION_ID)`.

The rest of the SKILL.md content (spawn protocol, layout discipline, deregister sequence) is unchanged.

### 7. Cross-Session Semantics (Structural Isolation)

Sessions are **namespaces, not security boundaries**. The invariant set:

1. `agents.session_id` is always set (NOT NULL FK into `sessions`).
2. `GET /ui/api/agents?session_id=<x>` returns only agents where `session_id = x`.
3. `POST /` JSON-RPC SendMessage resolves the caller's session via `SELECT session_id FROM agents WHERE agent_id = X-Agent-Id`, then refuses to deliver to any agent outside that session.
4. Broadcasts target exactly `active_agents WHERE session_id = <caller.session_id>`.
5. `tasks` inherit session membership through `context_id` → `agents.session_id`; reads like `GET /ui/api/agents/{id}/inbox` join via `agents` and enforce the `X-Session-Id` header matches.
6. There is no cross-session read/write surface. Discovering a session_id is sufficient to observe that session, but there is no endpoint that enumerates tasks or agents without a session_id (except the session list itself, which is deliberately public).

If a caller knows session A's ID, they can fully observe session A. That is the user's accepted risk: sessions are for tidiness, not for defense.

### 8. Coordination Note — 0000014 Member Lifecycle

`0000014-hikyaku-member-lifecycle` is **Approved** (0/44 tasks, frozen). It adds an `agent_placements` table (Alembic revision `0002_add_agent_placements`) and a `hikyaku member` CLI group, and currently assumes `agents.tenant_id` and `HIKYAKU_API_KEY`.

**Alembic ordering**: This document (0000015) claims Alembic revision `0002_local_simplification`. When 0000014 unfreezes, it must rebase its migration to `0003` with `down_revision = "0002_local_simplification"`.

**Shared cleanup**: Both documents currently claim the `Bash(printenv HIKYAKU_URL HIKYAKU_API_KEY)` removal from `.claude/settings.json` in their Success Criteria. **This document owns that edit** — it replaces the entry with `Bash(printenv HIKYAKU_URL HIKYAKU_SESSION_ID)` and adds `Bash(hikyaku-registry session *)`. When 0000014 unfreezes, its corresponding Success Criteria line should be removed as already-satisfied.

The member-lifecycle doc assumes `agents.tenant_id`. When 0000015 lands, that single word becomes `agents.session_id` throughout the member-lifecycle spec. The edit is a mechanical `tenant_id` → `session_id` rename; no structural changes are required:

- `agent_placements.director_agent_id` FK is unaffected.
- "Tenant scoping is implicit" language becomes "Session scoping is implicit" with the same join path.
- `HIKYAKU_API_KEY` in its tmux section becomes `HIKYAKU_SESSION_ID`.

### 9. Risks & Non-Goals

**Risks:**

| Risk | Mitigation |
|---|---|
| `0.0.0.0` bind accidentally exposes the broker on a shared network | Doc warning in README + `docs/` — user-owned decision, explicitly not code-gated per user direction |
| Migration seeding step misbehaves on a fresh DB that has `api_keys` rows created during `db init` sanity tests | `INSERT ... SELECT ... FROM api_keys` (no WHERE filter) is a no-op on empty tables; tested in `test_alembic_0002_upgrade.py` |
| The 0000014 parallel doc unfreezes and claims `0002` before this one | 0000014 is frozen (Approved, 0/44); this doc owns `0002`. When 0000014 unfreezes it rebases to `0003` |
| WebUI session picker is bypass-able by pasting a URL `/ui/#/sessions/<unknown-uuid>/agents` | `Dashboard` must refuse to render when `listSessions()` doesn't contain the URL's session_id — implemented as a guard in `App.tsx`'s hash parser |
| Tests that imported `get_user_id` / `verify_auth0_user` break in bulk during the cut | Delete `test_auth0.py` and `test_key_endpoints.py` outright; rewrite `test_webui_api.py`, `test_auth.py`, etc. as listed in Step 10 |
| `hikyaku-registry session delete` on a session with deregistered (but not purged) agents rows hits the `ondelete='RESTRICT'` — friendly error needed | The sync `DELETE` in `cli.py` catches `IntegrityError`, runs `SELECT COUNT(*) FROM agents WHERE session_id = ?`, and raises `click.UsageError` with the count |

**Non-goals:**

- Multi-user / multi-tenant semantics of any kind (explicitly out)
- Remote deployment hardening (explicitly out)
- OIDC / SSO / any alternative auth provider (explicitly out)
- Backwards compatibility with Auth0 webUI clients (explicitly out — no rollback path)
- Session metadata beyond `(session_id, label, created_at)` (no owner, no scopes, no expiry)
- Automatic session lifecycle management — no TTL, no cleanup job

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-04-12T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Implementation order is **documentation-first** per project rule `.claude/rules/design-doc-numbering.md`. Every `docs/`, `README.md`, `ARCHITECTURE.md`, and `SKILL.md` update must ship before any source code change. The task list is strictly ordered — do not reorder.

### Step 1: Documentation — user-facing

- [x] Update `README.md` — replace Auth0 references with session_id flow; rewrite feature bullet (line 16 "Two-Header Auth" → session-based), ASCII architecture diagram (22-46, drop `api_keys`, add `sessions`), design decisions (48-57), quickstart (call `hikyaku-registry session create` instead of Auth0 login), CLI table (135-153), WebUI API table (233-245), and delete "Running Vite dev server with Auth0" section (321-336); add bold "local-only tool, do not expose 0.0.0.0 on a shared network" warning <!-- completed: 2026-04-12T02:30 -->
- [x] Update `ARCHITECTURE.md` — redraw data model (drop `api_keys`, add `sessions`), rewrite "Tenant Isolation" section (33-55) to "Session Isolation" (no auth, `X-Session-Id` header, no Auth0), update component-layout table (`auth.py` description changes from "Auth0 JWT validation" to "session + agent-id resolution"), remove Auth0 from component diagram <!-- completed: 2026-04-12T02:30 -->
- [x] Update `docs/spec/data-model.md` — delete `ApiKey` entity and its Operation mapping section, add `Session` entity, rename `agents.tenant_id` → `agents.session_id`, mark `owner_sub` deleted, update Tenant Lifecycle to Session Lifecycle <!-- completed: 2026-04-12T02:30 -->
- [x] Update `docs/spec/registry-api.md` — remove bearer auth from all endpoints; add `session_id` body/query/header contract; add `SESSION_REQUIRED`/`SESSION_NOT_FOUND`/`AGENT_ID_REQUIRED` error codes; remove 401/403 from error table <!-- completed: 2026-04-12T02:30 -->
- [x] Update `docs/spec/webui-api.md` — delete Auth0 section, delete `/ui/api/auth/config` + `/ui/api/keys*` sections, add `GET /ui/api/sessions`, add `X-Session-Id` header to remaining endpoints; rewrite `/ui/api/timeline` section (lines 128-175) to use `X-Session-Id` instead of `X-Tenant-Id` (the endpoint is spec-only for blocked 0000013 but must stay consistent with the new model) <!-- completed: 2026-04-12T02:30 -->
- [x] Update `docs/spec/cli-options.md` — rename `HIKYAKU_API_KEY` to `HIKYAKU_SESSION_ID` globally; document `HIKYAKU_URL` 127.0.0.1 fallback; add `hikyaku-registry session` subcommands <!-- completed: 2026-04-12T02:30 -->
- [x] Update `docs/spec/a2a-operations.md` — update the JSON-RPC bearer header section to `X-Session-Id` + `X-Agent-Id` only; replace 401 errors with 400/404; change session-mismatch code from `-32001` to `-32003` <!-- completed: 2026-04-12T02:30 -->

### Step 2: Documentation — skills & settings

- [x] Update `.claude/skills/hikyaku/SKILL.md` — rename `HIKYAKU_API_KEY` → `HIKYAKU_SESSION_ID` globally; update Environment Variables section; update tmux spawn recipe; add `hikyaku-registry session create` bootstrap step to Typical Workflow <!-- completed: 2026-04-12T02:40 -->
- [x] Update `.claude/settings.json` — replace `Bash(printenv HIKYAKU_URL HIKYAKU_API_KEY)` with `Bash(printenv HIKYAKU_URL HIKYAKU_SESSION_ID)`; add `Bash(hikyaku-registry session *)` to the allow list <!-- completed: 2026-04-12T02:40 -->
- [x] Verify `.claude/settings.local.json` does not exist in the repo (confirmed absent at design time). If a developer's local copy exists at implementation time, grep it for `auth0|HIKYAKU_API_KEY` and remove hits; otherwise no-op <!-- completed: 2026-04-12T02:40 -->
- [x] Update `0000013-admin-discord-style-timeline/design-doc.md` — change Status line from `Approved` to `Blocked on 0000015`. No other edits. <!-- completed: 2026-04-12T02:40 -->

### Step 3: Backend — schema & migration

- [ ] Add `Session` model to `registry/src/hikyaku_registry/db/models.py` <!-- completed: -->
- [ ] Rename `Agent.tenant_id` → `Agent.session_id` in `db/models.py`; update FK target to `sessions.session_id`; rename `idx_agents_tenant_status` → `idx_agents_session_status` <!-- completed: -->
- [ ] Delete `ApiKey` model class from `db/models.py` <!-- completed: -->
- [ ] Write `registry/src/hikyaku_registry/alembic/versions/0002_local_simplification.py` per Specification §2 <!-- completed: -->
- [ ] Run `mise //registry:test` subset for `test_alembic_smoke.py` to verify the migration applies cleanly on a fresh DB <!-- completed: -->
- [ ] Add `test_alembic_0002_upgrade.py` that creates a 0001-schema DB, inserts an `api_keys` row + an `agents` row referencing it, runs the upgrade, and asserts: (a) `sessions` row exists with `session_id = api_key_hash`, (b) `agents.session_id` FK is valid, (c) `api_keys` table is gone <!-- completed: -->

### Step 4: Backend — registry store & session store

- [ ] Delete `RegistryStore.create_api_key`, `list_api_keys`, `revoke_api_key`, `get_api_key_status`, `is_api_key_active`, `is_key_owner` methods <!-- completed: -->
- [ ] Rename `RegistryStore.verify_agent_tenant` → `verify_agent_session` and update the query + all call sites <!-- completed: -->
- [ ] Update `RegistryStore.list_active_agents(tenant_id=...)` → `list_active_agents(session_id=...)` <!-- completed: -->
- [ ] Update `RegistryStore.create_agent` signature: drop `api_key` param, add `session_id` param; delete the sha256 derivation <!-- completed: -->
- [ ] Add async `RegistryStore.list_sessions` and `RegistryStore.get_session` methods for the WebUI runtime (`GET /ui/api/sessions`). CLI session management (create/list/show/delete) uses sync helpers in `cli.py` via `create_engine(sync_url)` — see Step 7 <!-- completed: -->
- [ ] Update `RegistryStore.list_deregistered_agents_with_tasks(tenant_id)` signature to `session_id` <!-- completed: -->
- [ ] Update `RegistryStore.get_agent_names` (no signature change; implementation unaffected) — audit only <!-- completed: -->

### Step 5: Backend — auth & middleware removal

- [ ] Delete `Auth0Verifier`, `verify_auth0_user`, `get_user_id` from `registry/src/hikyaku_registry/auth.py` <!-- completed: -->
- [ ] Delete `get_authenticated_agent`'s bearer-check path; replace with two new dependencies:
  - `get_session_from_agent_id(request, store)` — reads `X-Agent-Id` header, looks up `agents.session_id`, returns `(agent_id, session_id)`. Used by JSON-RPC and REST endpoints that identify the caller by agent (e.g., `DELETE /agents/{id}`, `POST /`)
  - `get_session_from_header(request, store)` — reads `X-Session-Id` header, verifies existence in `sessions` table, returns `session_id`. Used by REST endpoints that do not identify a caller agent (e.g., `GET /agents`, `GET /agents/{id}`)
  <!-- completed: -->
- [ ] Delete `get_registration_tenant` entirely <!-- completed: -->
- [ ] Delete `auth0_domain`, `auth0_client_id`, `auth0_audience` from `config.py` <!-- completed: -->
- [ ] Remove PyJWT from `registry/pyproject.toml` dependencies; run `uv sync` <!-- completed: -->

### Step 6: Backend — HTTP routes

- [ ] Rewrite `registry/src/hikyaku_registry/api/registry.py`:
  - `POST /agents` accepts `{session_id, name, description, skills}` in the body
  - `GET /agents` reads `?session_id=` query param
  - `GET /agents/{id}` reads `X-Session-Id` header
  - `DELETE /agents/{id}` reads `X-Agent-Id` header only
  - All 401 bearer errors become 400 `SESSION_REQUIRED` / 404 `SESSION_NOT_FOUND`
  <!-- completed: -->
- [ ] Rewrite `registry/src/hikyaku_registry/webui_api.py`:
  - Delete `GET /ui/api/auth/config`, `POST /ui/api/keys`, `GET /ui/api/keys`, `DELETE /ui/api/keys/{id}`
  - Delete `get_webui_tenant` dependency; add `get_webui_session` that reads `X-Session-Id` header and verifies existence
  - Add `GET /ui/api/sessions` endpoint (calls async `RegistryStore.list_sessions`, no session header required)
  - Update `/ui/api/agents`, `/ui/api/agents/{id}/inbox`, `/ui/api/agents/{id}/sent`, `/ui/api/messages/send` to use `get_webui_session`
  - All `_get_tenant_agents` helpers renamed `_get_session_agents`
  <!-- completed: -->
- [ ] Rewrite `registry/src/hikyaku_registry/main.py`:
  - Delete the bearer extraction + `is_api_key_active` check in the `POST /` JSON-RPC endpoint
  - Replace with `X-Agent-Id` header → agents table lookup → resolve `session_id`
  - Update `_handle_send_message`, `_handle_get_task`, `_handle_cancel_task` to pass `session_id` through `call_context.state` instead of `tenant_id` (`_handle_list_tasks` does not use tenant_id today — no change needed)
  <!-- completed: -->
- [ ] Update `registry/src/hikyaku_registry/executor.py`:
  - Define `class SessionMismatchError(ValueError)` at module level (executor.py is its home module — other exception classes like routing errors already live here)
  - Rename every `tenant_id` local to `session_id`
  - `_handle_unicast` session-mismatch path (currently `raise ValueError(...)` at line 111) raises `SessionMismatchError("Session mismatch")` instead of plain `ValueError`
  <!-- completed: -->
- [ ] Update `registry/src/hikyaku_registry/main.py` exception handler:
  - In `jsonrpc_endpoint`'s `except` block (line 331), add a specific `except SessionMismatchError` catch **before** the existing `except (ValueError, PermissionError)` catch that maps to `-32000`
  - The new catch returns `_jsonrpc_error(-32003, str(e), req_id)` — this is how the `-32003` code reaches the JSON-RPC response
  - Import `SessionMismatchError` from `executor`
  <!-- completed: -->

### Step 7: Backend — `hikyaku-registry session` CLI

- [ ] Add `@main.group() def session()` to `registry/src/hikyaku_registry/cli.py` <!-- completed: -->
- [ ] Implement `session create [--label TEXT] [--json]`: open sync engine via `create_engine(_sync_db_url())`, `uuid4()` → `INSERT INTO sessions`, print UUID on stdout <!-- completed: -->
- [ ] Implement `session list [--json]`: sync `SELECT ... LEFT JOIN agents` + count agents per session; table output by default <!-- completed: -->
- [ ] Implement `session show <session_id> [--json]`: sync `SELECT` from sessions; exit 1 with friendly error if missing <!-- completed: -->
- [ ] Implement `session delete <session_id>`: sync `DELETE FROM sessions`; catch `IntegrityError`, query agent count, raise `click.UsageError` <!-- completed: -->
- [ ] Verify `db init` remains unchanged and does NOT auto-create a session <!-- completed: -->

### Step 8: Client — CLI & api rename

- [ ] Rename every `HIKYAKU_API_KEY` → `HIKYAKU_SESSION_ID` in `client/src/hikyaku_client/cli.py` (env var read, `_require_api_key` → `_require_session_id`, error message, help text) <!-- completed: -->
- [ ] Update `client/src/hikyaku_client/api.py`: replace `Authorization: Bearer` header with `X-Session-Id` header on every request; rename `api_key` parameters to `session_id`; add `session_id` to `register_agent` POST body; add `?session_id=` query param to `list_agents` GET request <!-- completed: -->
- [ ] Change default URL fallback in `cli.py` from `http://localhost:8000` to `http://127.0.0.1:8000` <!-- completed: -->
- [ ] Update `register` command: add `_require_session_id(ctx)` at the function entry (currently the only command without a key-requirement check — register used to mint the key itself), send `session_id` in the POST body, and set the `X-Session-Id` header (same code path as every other command after the rename) <!-- completed: -->
- [ ] Verify `client/pyproject.toml` has **no** SQLAlchemy / aiosqlite dependency added <!-- completed: -->

### Step 9: Admin webUI

- [ ] Delete `admin/src/components/LoginPage.tsx` <!-- completed: -->
- [ ] Delete `admin/src/components/KeyManagement.tsx` <!-- completed: -->
- [ ] Remove `@auth0/auth0-react` from `admin/package.json`; run `bun install` to update `bun.lock` <!-- completed: -->
- [ ] Delete `VITE_AUTH0_REDIRECT_URI` line from `admin/mise.toml` <!-- completed: -->
- [ ] Rewrite `admin/src/App.tsx`:
  - Remove `Auth0Provider`, `useAuth0`, `getAuthConfig`, `tokenReady` state
  - Add hash-based routing: parse `window.location.hash`, handle `hashchange`
  - Route `/ui/#/sessions` → `<SessionPicker>`
  - Route `/ui/#/sessions/<uuid>/agents` → `<Dashboard sessionId={...}>`
  - Guard: if URL session_id is not in `listSessions()` response, redirect to picker
  <!-- completed: -->
- [ ] Rewrite `admin/src/api.ts`:
  - Delete `setGetAccessToken`, `getAccessToken` closure, `getAuthConfig`, `createKey`, `listKeys`, `revokeKey`
  - Rename `setTenantId` → `setSessionId`, `getTenantId` → `getSessionId` (these replace both the `getAccessToken` closure at line 9 AND the `tenantId` closure at lines 10/16/20 — two closures become one)
  - Update `request<T>` to emit `X-Session-Id` instead of both `Authorization: Bearer` (line 34) and `X-Tenant-Id` (line 38)
  - Add `listSessions(): Promise<SessionListItem[]>`
  <!-- completed: -->
- [ ] Update `admin/src/types.ts`: delete `ApiKey`, `CreateKeyResponse`; add `SessionListItem` <!-- completed: -->
- [ ] Add `admin/src/components/SessionPicker.tsx` per Specification §5.2 (must include empty-state hint: "Run `hikyaku-registry session create` to create one") <!-- completed: -->
- [ ] Rename `Dashboard` prop `tenantId` → `sessionId` and propagate through `AgentTabs`, `MessageList`, `SendMessageForm`, `MessageRow` <!-- completed: -->
- [ ] Run `mise //admin:lint` and `mise //admin:build`; rebuild the bundled webUI into `registry/src/hikyaku_registry/webui/` per 0000012 flow <!-- completed: -->

### Step 10: Tests

- [ ] Delete `registry/tests/test_auth0.py` <!-- completed: -->
- [ ] Delete `registry/tests/test_key_endpoints.py` <!-- completed: -->
- [ ] Rewrite `registry/tests/test_auth.py` — drop bearer-key scenarios, add `X-Session-Id`/`X-Agent-Id` header scenarios <!-- completed: -->
- [ ] Rewrite `registry/tests/test_registry_api.py` — replace api_key fixtures with session fixtures; update all `tenant_id` asserts to `session_id`; add cross-session `GET /api/v1/agents/{id}` returns 404 test <!-- completed: -->
- [ ] Rewrite `registry/tests/test_registry_store.py` — drop `create_api_key`/`list_api_keys`/`revoke_api_key` tests; add async `list_sessions`/`get_session` tests (sync session CRUD is covered by `test_session_cli.py`) <!-- completed: -->
- [ ] Rewrite `registry/tests/test_executor.py` — rename `tenant_id` → `session_id`; add cross-session send rejection case asserting JSON-RPC error `-32003` <!-- completed: -->
- [ ] Rewrite `registry/tests/test_webui_api.py` — drop Auth0 mocks; use plain `X-Session-Id` header; assert `/ui/api/keys*` endpoints return 404; add cross-session `GET /ui/api/agents/{id}/inbox` returns 404 test <!-- completed: -->
- [ ] Delete `registry/tests/test_webui_auth_migration.py` (file is exclusively JWT + X-Tenant-Id Auth0 coverage; no longer applicable) <!-- completed: -->
- [ ] Rewrite `registry/tests/test_db_models.py` — drop `ApiKey` tests, add `Session` tests, rename `tenant_id` asserts <!-- completed: -->
- [ ] Update `registry/tests/test_alembic_smoke.py` — head revision changes from `0001` to `0002_local_simplification`; assert new table list (`agents`, `sessions`, `tasks`, `alembic_version`) <!-- completed: -->
- [ ] Update `registry/tests/test_db_init.py` — update both `expected` table sets at lines 109 and 160 from `{"api_keys", "agents", "tasks", "alembic_version"}` to `{"sessions", "agents", "tasks", "alembic_version"}` <!-- completed: -->
- [ ] Rewrite `registry/tests/test_task_store.py` — replace `store.create_api_key(owner_sub)` fixture pattern (line 130) with `store.create_session(label=...)`; update `create_agent` calls to pass `session_id` instead of `api_key`; rename "two owners → two api_keys" docstring language to "two sessions" <!-- completed: -->
- [ ] Update `registry/tests/test_a2a.py` — adjust any tenant/bearer helpers <!-- completed: -->
- [ ] Add `registry/tests/test_session_cli.py` — click.testing.CliRunner coverage for `session create/list/show/delete` including the FK-violation error path <!-- completed: -->
- [ ] Update `client/tests/test_cli.py` and `client/tests/test_cli_register.py` — replace `HIKYAKU_API_KEY` env fixtures with `HIKYAKU_SESSION_ID`; assert `X-Session-Id` header on outgoing requests <!-- completed: -->

### Step 11: Final verification

- [ ] Run `mise //:lint` — must pass <!-- completed: -->
- [ ] Run `mise //:format` — must pass <!-- completed: -->
- [ ] Run `mise //:typecheck` — must pass <!-- completed: -->
- [ ] Run `mise //registry:test` — must pass <!-- completed: -->
- [ ] Run `mise //client:test` — must pass <!-- completed: -->
- [ ] Run `mise //admin:lint` — must pass <!-- completed: -->
- [ ] Run `mise //admin:build` — must pass <!-- completed: -->
- [ ] Manual smoke: `hikyaku-registry db init`, `hikyaku-registry session create --label test`, `mise //registry:dev`, open `http://127.0.0.1:8000/ui/`, pick the test session, verify empty agents list renders <!-- completed: -->
- [ ] Manual smoke: in a second terminal, `export HIKYAKU_SESSION_ID=<uuid>`, `hikyaku register --name A --description a`, `hikyaku register --name B --description b`, `hikyaku send --agent-id <A> --to <B> --text hi`, `hikyaku poll --agent-id <B>` — verify delivery <!-- completed: -->
- [ ] Grep for residual references: `grep -rn AUTH0 registry/ admin/ client/ docs/ README.md ARCHITECTURE.md` must return zero hits; same for `auth0_domain`, `api_key_hash`, `owner_sub`, `HIKYAKU_API_KEY`, `verify_auth0_user` <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-12 | Initial draft |
| 2026-04-12 | R1 revision: fix session_id format (opaque string, not UUIDv4-only); fix migration seeding (all api_keys, not just active); remove FK drop_constraint (batch mode handles it); fix -32001 → -32003 collision; remove 401 from error taxonomy; split auth dependencies (get_session_from_agent_id / get_session_from_header); fix 0000014 status (Approved, not Draft) + settings.json overlap ownership; fix SKILL.md line ranges; remove phantom access-control.md task; move /ui/api/sessions handler to webui_api.py; add cross-session HTTP isolation tests; clarify sync CLI vs async runtime session APIs; add empty SessionPicker UX hint; explicit README/ARCHITECTURE scope; misc wording fixes |
| 2026-04-12 | R1 addendum (C7+C8): add SessionMismatchError exception class in executor.py + main.py catch wiring for -32003; fix §3 table residual -32001→-32003; clarify X-Tenant-Id removal scope in §3 table, §5.1, Step 9 (admin sends both Authorization AND X-Tenant-Id — both replaced by X-Session-Id); split Step 6 executor.py + main.py tasks; 0/79→0/80 |
