# Remove Auth0: Local-Only Session Model

**Status**: Draft
**Progress**: 0/81 tasks complete
**Last Updated**: 2026-04-12

## Overview

Collapse Hikyaku's multi-tenant access-control model into a single local-only namespace. Auth0, tenants, and API keys are removed end-to-end; a non-secret `session_id` (UUIDv4) becomes the sole namespace key, minted via a new `hikyaku-registry session create` administrative command that writes directly to SQLite. The broker runs without authentication on the assumption that only the local developer can reach it.

## Success Criteria

- [ ] `@auth0/auth0-react`, `PyJWT` JWKS flow, `Auth0Verifier`, `verify_auth0_user`, `get_user_id`, and every `VITE_AUTH0_*` / `AUTH0_*` env var are fully deleted from the codebase
- [ ] `api_keys` table, `owner_sub` column, `ApiKey` model, `is_key_owner`, `create_api_key`, `list_api_keys`, `revoke_api_key`, and `is_api_key_active` are fully deleted
- [ ] New `sessions` table exists with schema `(session_id UUID PK, label TEXT NULL, created_at TEXT NOT NULL)`, declared in `db/models.py` and created by Alembic migration `0002_local_simplification`
- [ ] `agents.tenant_id` column is renamed to `session_id` in the same migration; FK retargets `sessions.session_id`; index `idx_agents_tenant_status` is renamed to `idx_agents_session_status`
- [ ] `hikyaku-registry session create [--label <text>]` mints a session by writing a row directly to SQLite (no HTTP) and prints the `session_id` in both human and `--json` modes
- [ ] `hikyaku-registry session list | show <id> | delete <id>` are implemented alongside `create`
- [ ] `hikyaku` client CLI reads `HIKYAKU_SESSION_ID` (not `HIKYAKU_API_KEY`) and forwards it to the broker via `X-Session-Id` header
- [ ] `HIKYAKU_URL` falls back to `http://127.0.0.1:8000` in the `hikyaku` client CLI when unset
- [ ] `client/` workspace has **no** SQLAlchemy dependency (admin ops live in `hikyaku-registry` only)
- [ ] Broker `POST /` JSON-RPC, `POST/GET/DELETE /api/v1/agents*`, and every `/ui/api/*` endpoint use `X-Session-Id` and do not require a bearer token
- [ ] Cross-session sends are rejected with HTTP 400 (`SESSION_MISMATCH`); cross-session reads return HTTP 404
- [ ] `GET /ui/api/auth/config`, `POST /ui/api/keys`, `GET /ui/api/keys`, `DELETE /ui/api/keys/{tenant_id}` are deleted
- [ ] WebUI has no login screen; first-load lands on a session picker at `/ui/#/sessions`; selecting a session navigates to `/ui/#/sessions/<uuid>/agents`
- [ ] `GET /ui/api/sessions` exists and returns all rows from the `sessions` table
- [ ] `registry/tests/test_auth0.py` is deleted
- [ ] `registry/tests/test_key_endpoints.py` is deleted
- [ ] Documentation updates ship **before** any code changes: `README.md`, `ARCHITECTURE.md`, `docs/spec/data-model.md`, `docs/spec/registry-api.md`, `docs/spec/webui-api.md`, `docs/spec/cli-options.md`, `docs/spec/a2a-operations.md`, `.claude/skills/hikyaku/SKILL.md`
- [ ] `docs/spec/access-control.md` is deleted (if present) or rewritten to point at this document
- [ ] `.claude/settings.json` no longer contains `Bash(printenv HIKYAKU_URL HIKYAKU_API_KEY)`
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
| SKILL.md's tmux spawn recipe already treats the API key as a plain env var passed between panes — the "secret" is ceremonial | `.claude/skills/hikyaku/SKILL.md:192-226` |

The user has reframed the project: `session_id` is just a non-secret namespace, the broker trusts its local network, key listing is fine because there is no secret to protect, and session creation belongs with the existing `hikyaku-registry db init` administrative CLI rather than the runtime `hikyaku` client.

### Design docs this supersedes

This document supersedes `design-docs/0000002-access-control/design-doc.md` and `design-docs/0000007-api-key-specification/design-doc.md`. Per user direction, those files are NOT edited in place — the supersede relationship lives only here, to minimize churn on completed historical docs.

### Related in-flight work

`design-docs/0000014-hikyaku-member-lifecycle/design-doc.md` is a parallel Draft that adds an `agent_placements` table and a `hikyaku member` CLI group. It currently assumes `agents.tenant_id` and `HIKYAKU_API_KEY`. The two efforts are designed to proceed **independently** (see Coordination Note below); there is no ordering dependency, only a mechanical 1-line schema rename.

`design-docs/0000013-admin-discord-style-timeline/design-doc.md` is Approved but 0/38 tasks and assumes Auth0 + tenant-scoped keys. Its Status will be flipped to `Blocked on 0000015` in a single-line edit; the body is not touched.

---

## Specification

### 1. Data Model

#### 1.1 New table: `sessions`

```python
class Session(Base):
    __tablename__ = "sessions"

    session_id: Mapped[str] = mapped_column(String, primary_key=True)  # UUIDv4 canonical form
    label: Mapped[str | None] = mapped_column(String, nullable=True)
    created_at: Mapped[str] = mapped_column(String, nullable=False)    # ISO 8601 UTC
```

Constraints:
- `session_id` is the canonical UUIDv4 string (`550e8400-e29b-41d4-a716-446655440000`) — generated server-side by `uuid.uuid4()`.
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

    # 2. Seed one synthetic session per distinct active api_keys row, using
    #    the api_key_hash as the session_id. This preserves existing
    #    agents.tenant_id values byte-for-byte so step 3 becomes a pure rename.
    op.execute(
        """
        INSERT INTO sessions (session_id, label, created_at)
        SELECT api_key_hash, 'legacy-' || key_prefix, created_at
        FROM api_keys
        WHERE status = 'active'
        """
    )

    # 3. Rename agents.tenant_id -> session_id, drop old FK + index, add
    #    new FK into sessions, create new index. SQLite requires batch mode
    #    for FK manipulation.
    with op.batch_alter_table("agents", schema=None) as batch_op:
        batch_op.drop_index("idx_agents_tenant_status")
        batch_op.alter_column("tenant_id", new_column_name="session_id")
        batch_op.drop_constraint("fk_agents_tenant", type_="foreignkey")
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
- The FK names (`fk_agents_tenant`, `fk_agents_session`) are illustrative; the actual drop/create uses whatever name Alembic autogenerates from the 0001 migration (SQLite does not store FK names, so `batch_alter_table` rewrites the full table regardless).
- The seeding step in (2) is necessary so the FK flip in (3) does not violate integrity on existing data. Fresh databases have zero `api_keys` rows and the INSERT is a no-op.
- `downgrade()` raises immediately — Alembic treats this as a one-way revision. This is called out in the docstring so future operators do not assume rollback is possible.

### 3. Broker HTTP Surface

| Path | Method | Before | After |
|---|---|---|---|
| `/api/v1/agents` | POST | Bearer API key → `tenant_id` from hash; body `{name, description, skills}` | **No bearer.** Body `{session_id, name, description, skills}`. Server 404s if `session_id` does not exist in `sessions` table. |
| `/api/v1/agents` | GET | Tenant from bearer | Query param `?session_id=<uuid>` (required). Lists agents in that session. |
| `/api/v1/agents/{id}` | GET | Bearer + tenant-scoped 404 | **No bearer.** Header `X-Session-Id: <uuid>` (required). 404 if agent missing OR agent.session_id ≠ header. |
| `/api/v1/agents/{id}` | DELETE | Bearer + self-check | **No bearer.** Header `X-Agent-Id: {id}` (self-identification). Returns 204 on success. Session is not re-verified. |
| `/` (JSON-RPC) | POST | Bearer + `X-Agent-Id` | **No bearer.** Header `X-Agent-Id: <caller>`. Broker resolves caller's session via `SELECT session_id FROM agents WHERE agent_id = ?`. Cross-session sends rejected with JSON-RPC error `-32001` "session mismatch". |
| `/ui/api/sessions` | GET | — | **New.** Returns `[{session_id, label, created_at, agent_count}, ...]`. No auth, no header required. |
| `/ui/api/agents` | GET | Auth0 + `X-Tenant-Id` | **No Auth0.** Query param `?session_id=<uuid>`. |
| `/ui/api/agents/{id}/inbox` | GET | Auth0 + `X-Tenant-Id` | **No Auth0.** Header `X-Session-Id: <uuid>`. |
| `/ui/api/agents/{id}/sent` | GET | Auth0 + `X-Tenant-Id` | **No Auth0.** Header `X-Session-Id: <uuid>`. |
| `/ui/api/messages/send` | POST | Auth0 + `X-Tenant-Id` | **No Auth0.** Header `X-Session-Id: <uuid>`. |
| `/ui/api/auth/config` | GET | Auth0 settings | **Deleted.** |
| `/ui/api/keys` | POST/GET | Auth0-scoped key CRUD | **Deleted.** |
| `/ui/api/keys/{id}` | DELETE | Auth0-scoped revoke | **Deleted.** |

#### 3.1 Error taxonomy

| Scenario | HTTP status | JSON body |
|---|---|---|
| `session_id` missing from required header/body/query | 400 | `{"error":{"code":"SESSION_REQUIRED","message":"..."}}` |
| `session_id` does not exist in `sessions` table | 404 | `{"error":{"code":"SESSION_NOT_FOUND","message":"..."}}` |
| Unicast target in a different session than caller | 404 | `{"error":{"code":"AGENT_NOT_FOUND","message":"..."}}` (indistinguishable from "missing" by design) |
| JSON-RPC send from agent A to agent B across sessions | JSON-RPC error | `{"code":-32001,"message":"Session mismatch"}` |
| `X-Agent-Id` on JSON-RPC does not exist | 401 | `{"error":"Unauthorized"}` (preserved from existing 401 behavior, but no bearer check) |

The code `-32001` is collision-free in the current codebase: `registry/src/hikyaku_registry/main.py:328,332` uses only `-32601` (method not found) and `-32000` (generic server error), and `-32001` is inside the JSON-RPC 2.0 reserved server-error range (`-32099` to `-32000`).

Rationale: The broker no longer performs authentication. It performs **namespace routing** — `session_id` is a filter, not a credential. Cross-session 404 rather than 403 is deliberate: a caller who knows a target's session_id can always observe it (there is no secret to protect), but the default responses keep sessions structurally isolated so accidental cross-session traffic produces the same shape of error as "the agent does not exist at all."

#### 3.2 Bind address

`broker_host` default remains `0.0.0.0`. **No** code-level guard rail, startup check, or `--force-public` flag. Instead, a prominent warning is added to `README.md` and `docs/spec/` documenting that Hikyaku is a local-only tool and binding to a public interface is a user-owned decision. Any user who binds `0.0.0.0` on a shared network is accepting that every listener can see and act within every session.

### 4. CLI

#### 4.1 `hikyaku-registry session` — new subcommand group

Lives in `registry/src/hikyaku_registry/cli.py` alongside the existing `db init` command. Opens the SQLite file directly using the sync engine already available to `db init`, so the broker server does not need to be running.

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

No `--json` toggle on the click group — add per-command where needed (`create`, `list`, `show`), mirroring how `hikyaku-client` does it today.

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

`client/src/hikyaku_client/cli.py` and `client/src/hikyaku_client/api.py` change in two ways:

1. **Env var rename**: Every reference to `HIKYAKU_API_KEY` → `HIKYAKU_SESSION_ID`. Error messages updated. The CLI exits with `Error: HIKYAKU_SESSION_ID environment variable is required. Create a session with 'hikyaku-registry session create'.` when unset.

2. **HTTP header rename**: `api.py` sends `X-Session-Id: <value>` instead of `Authorization: Bearer <value>`. The `X-Agent-Id` header continues to identify the caller.

3. **URL fallback**: `HIKYAKU_URL` defaults to `http://127.0.0.1:8000` when unset (previously `http://localhost:8000` — tighten to loopback to match the local-only stance).

4. **NO new dependencies**: `client/` remains HTTP-only. SQLAlchemy/aiosqlite stay in `registry/` only. `hikyaku session create` does NOT exist on the runtime CLI.

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
| `admin/src/api.ts` | Remove `setGetAccessToken`, `getAccessToken` closure, `getAuthConfig`, `createKey`, `listKeys`, `revokeKey`. Replace `Authorization` header with `X-Session-Id` header. Add `listSessions()`. |
| `admin/package.json` | Remove `@auth0/auth0-react` dependency; run `bun install` to update `bun.lock`. |
| `admin/mise.toml` | Delete the `VITE_AUTH0_REDIRECT_URI` env entry (line 6). |
| `admin/.env*` (any Auth0 lines) | Delete |

#### 5.2 Additions

**New component**: `admin/src/components/SessionPicker.tsx`

```tsx
// Sketch — full implementation in Step 5.
export default function SessionPicker({
  onSelect,
}: {
  onSelect: (sessionId: string) => void;
}) {
  const [sessions, setSessions] = useState<SessionListItem[]>([]);
  useEffect(() => {
    listSessions().then(setSessions);
  }, []);
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
| `/ui/#/sessions/<uuid>/agents` | `<Dashboard tenantId={...} />` — parameter renamed `sessionId` |
| `/ui/#/sessions/<uuid>/agents/<agent_id>/inbox` | Agent inbox tab |
| `/ui/#/sessions/<uuid>/agents/<agent_id>/sent` | Agent sent tab |

`App.tsx` parses `window.location.hash` on mount and on `hashchange`. No external router library (`react-router`) is added — the existing codebase has no router today, and a two-screen hash parser keeps the SPA bundle small.

`api.ts`'s `setTenantId` function is renamed `setSessionId`. Every outgoing `/ui/api/*` request includes the `X-Session-Id: <active>` header.

### 6. tmux Multi-Session Coordination

The flow in `.claude/skills/hikyaku/SKILL.md:192-226` is mechanically equivalent — only the env var name changes. The new recipe:

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

`0000014-hikyaku-member-lifecycle` (Draft) is being developed in parallel. The two efforts are **independent** — neither blocks the other — but they share schema real estate. Whichever lands first claims Alembic revision `0002`; the other rebases its migration file to `0003` before merge.

The member-lifecycle doc assumes `agents.tenant_id`. When 0000015 lands, that single word becomes `agents.session_id` throughout the member-lifecycle spec. The edit is a mechanical `tenant_id` → `session_id` rename; no structural changes are required:

- `agent_placements.director_agent_id` FK is unaffected.
- "Tenant scoping is implicit" language becomes "Session scoping is implicit" with the same join path.
- `HIKYAKU_API_KEY` in its tmux section becomes `HIKYAKU_SESSION_ID`.

Neither team must wait for the other. The person landing second reconciles mechanically. This is recorded here so both teams know the plan without requiring a shared owner.

### 9. Risks & Non-Goals

**Risks:**

| Risk | Mitigation |
|---|---|
| `0.0.0.0` bind accidentally exposes the broker on a shared network | Doc warning in README + `docs/` — user-owned decision, explicitly not code-gated per user direction |
| Migration seeding step misbehaves on a fresh DB that has `api_keys` rows created during `db init` sanity tests | `INSERT ... SELECT ... FROM api_keys WHERE status='active'` is a no-op on empty tables; tested in `test_alembic_0002_upgrade.py` |
| The 0000014 parallel doc merges first and claims `0002` before this one | Each side detects by checking `alembic/versions/` at merge time; loser renames their file to `0003_*` and updates `down_revision` |
| WebUI session picker is bypass-able by pasting a URL `/ui/#/sessions/<unknown-uuid>/agents` | `Dashboard` must refuse to render when `listSessions()` doesn't contain the URL's session_id — implemented as a guard in `App.tsx`'s hash parser |
| Tests that imported `get_user_id` / `verify_auth0_user` break in bulk during the cut | Delete `test_auth0.py` outright and rewrite `test_webui_api.py`, `test_auth.py`, `test_key_endpoints.py`, etc. as listed in Step 6 |
| `hikyaku-registry session delete` on a session with deregistered (but not purged) agents rows hits the `ondelete='RESTRICT'` — friendly error needed | `session_store.delete_session` catches `IntegrityError`, runs a SELECT COUNT against agents, and raises `click.UsageError` with the count |

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

- [ ] Update `README.md` — replace Auth0 references with session_id flow; add bold "local-only tool, do not expose 0.0.0.0" warning; update quickstart to call `hikyaku-registry session create` <!-- completed: -->
- [ ] Update `ARCHITECTURE.md` — redraw data model (drop api_keys, add sessions), update broker auth section (no auth, `X-Session-Id` header), remove Auth0 from component diagram <!-- completed: -->
- [ ] Update `docs/spec/data-model.md` — delete `ApiKey` entity, add `Session` entity, rename `agents.tenant_id` → `agents.session_id`, mark `owner_sub` deleted <!-- completed: -->
- [ ] Update `docs/spec/registry-api.md` — remove bearer auth from all endpoints; add `session_id` body/query/header contract; add `SESSION_REQUIRED`/`SESSION_NOT_FOUND` error codes <!-- completed: -->
- [ ] Update `docs/spec/webui-api.md` — delete Auth0 section, delete `/ui/api/auth/config` + `/ui/api/keys*` sections, add `GET /ui/api/sessions`, add `X-Session-Id` header to remaining endpoints <!-- completed: -->
- [ ] Update `docs/spec/cli-options.md` — rename `HIKYAKU_API_KEY` to `HIKYAKU_SESSION_ID` globally; document `HIKYAKU_URL` 127.0.0.1 fallback; add `hikyaku-registry session` subcommands <!-- completed: -->
- [ ] Update `docs/spec/a2a-operations.md` — update the JSON-RPC bearer header section to `X-Session-Id` + `X-Agent-Id` only <!-- completed: -->
- [ ] Delete or rewrite `docs/spec/access-control.md` if it exists (redirect note pointing at this document) <!-- completed: -->

### Step 2: Documentation — skills & settings

- [ ] Update `.claude/skills/hikyaku/SKILL.md` — rename `HIKYAKU_API_KEY` → `HIKYAKU_SESSION_ID` globally; update Environment Variables section; update tmux spawn recipe; add `hikyaku-registry session create` bootstrap step to Typical Workflow <!-- completed: -->
- [ ] Update `.claude/settings.json` — replace `Bash(printenv HIKYAKU_URL HIKYAKU_API_KEY)` with `Bash(printenv HIKYAKU_URL HIKYAKU_SESSION_ID)`; add `Bash(hikyaku-registry session:*)` if a scoped pattern is needed <!-- completed: -->
- [ ] Verify `.claude/settings.local.json` does not exist in the repo (confirmed absent at design time). If a developer's local copy exists at implementation time, grep it for `auth0|HIKYAKU_API_KEY` and remove hits; otherwise no-op <!-- completed: -->
- [ ] Update `0000013-admin-discord-style-timeline/design-doc.md` — change Status line from `Approved` to `Blocked on 0000015`. No other edits. <!-- completed: -->

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
- [ ] Add `RegistryStore.create_session`, `list_sessions`, `get_session`, `delete_session` methods with the schemas in Specification §4.1. `delete_session` must raise `click.UsageError`-compatible exception on FK violation <!-- completed: -->
- [ ] Update `RegistryStore.list_deregistered_agents_with_tasks(tenant_id)` signature to `session_id` <!-- completed: -->
- [ ] Update `RegistryStore.get_agent_names` (no signature change; implementation unaffected) — audit only <!-- completed: -->

### Step 5: Backend — auth & middleware removal

- [ ] Delete `Auth0Verifier`, `verify_auth0_user`, `get_user_id` from `registry/src/hikyaku_registry/auth.py` <!-- completed: -->
- [ ] Delete `get_authenticated_agent`'s bearer-check path; replace with `get_agent_and_session(request)` that reads `X-Agent-Id`, looks up `agents.session_id`, and returns `(agent_id, session_id)` <!-- completed: -->
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
- [ ] Add a new file `registry/src/hikyaku_registry/api/sessions.py` with `GET /ui/api/sessions` endpoint (returns `list_sessions` result) <!-- completed: -->
- [ ] Rewrite `registry/src/hikyaku_registry/webui_api.py`:
  - Delete `GET /ui/api/auth/config`, `POST /ui/api/keys`, `GET /ui/api/keys`, `DELETE /ui/api/keys/{id}`
  - Delete `get_webui_tenant` dependency; add `get_webui_session` that reads `X-Session-Id` header and verifies existence
  - Update `/ui/api/agents`, `/ui/api/agents/{id}/inbox`, `/ui/api/agents/{id}/sent`, `/ui/api/messages/send` to use `get_webui_session`
  - All `_get_tenant_agents` helpers renamed `_get_session_agents`
  <!-- completed: -->
- [ ] Rewrite `registry/src/hikyaku_registry/main.py`:
  - Delete the bearer extraction + `is_api_key_active` check in the `POST /` JSON-RPC endpoint
  - Replace with `X-Agent-Id` header → agents table lookup → resolve `session_id`
  - Update `_handle_send_message`, `_handle_get_task`, `_handle_cancel_task`, `_handle_list_tasks` to pass `session_id` through `call_context.state` instead of `tenant_id`
  <!-- completed: -->
- [ ] Update `registry/src/hikyaku_registry/executor.py`:
  - Rename every `tenant_id` local to `session_id`
  - `_handle_unicast` session-mismatch path raises JSON-RPC error `-32001 "Session mismatch"`
  <!-- completed: -->

### Step 7: Backend — `hikyaku-registry session` CLI

- [ ] Add `@main.group() def session()` to `registry/src/hikyaku_registry/cli.py` <!-- completed: -->
- [ ] Implement `session create [--label TEXT] [--json]`: uuid4 → `RegistryStore.create_session` → print UUID on stdout <!-- completed: -->
- [ ] Implement `session list [--json]`: call `list_sessions` + count agents per session; table output by default <!-- completed: -->
- [ ] Implement `session show <session_id> [--json]`: call `get_session`; exit 1 with friendly error if missing <!-- completed: -->
- [ ] Implement `session delete <session_id>`: call `delete_session`; catch `IntegrityError`, query agent count, raise `click.UsageError` <!-- completed: -->
- [ ] Verify `db init` remains unchanged and does NOT auto-create a session <!-- completed: -->

### Step 8: Client — CLI & api rename

- [ ] Rename every `HIKYAKU_API_KEY` → `HIKYAKU_SESSION_ID` in `client/src/hikyaku_client/cli.py` (env var read, `_require_api_key` → `_require_session_id`, error message, help text) <!-- completed: -->
- [ ] Update `client/src/hikyaku_client/api.py`: replace `Authorization: Bearer` header with `X-Session-Id` header on every request; rename `api_key` parameters to `session_id` <!-- completed: -->
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
  - Delete `setGetAccessToken`, `getAuthConfig`, `createKey`, `listKeys`, `revokeKey`
  - Rename `setTenantId` → `setSessionId`, `getTenantId` → `getSessionId`
  - Update `request<T>` to emit `X-Session-Id` instead of `Authorization`
  - Add `listSessions(): Promise<SessionListItem[]>`
  <!-- completed: -->
- [ ] Update `admin/src/types.ts`: delete `ApiKey`, `CreateKeyResponse`; add `SessionListItem` <!-- completed: -->
- [ ] Add `admin/src/components/SessionPicker.tsx` per Specification §5.2 <!-- completed: -->
- [ ] Rename `Dashboard` prop `tenantId` → `sessionId` and propagate through `AgentTabs`, `MessageList`, `SendMessageForm`, `MessageRow` <!-- completed: -->
- [ ] Run `mise //admin:lint` and `mise //admin:build`; rebuild the bundled webUI into `registry/src/hikyaku_registry/webui/` per 0000012 flow <!-- completed: -->

### Step 10: Tests

- [ ] Delete `registry/tests/test_auth0.py` <!-- completed: -->
- [ ] Delete `registry/tests/test_key_endpoints.py` <!-- completed: -->
- [ ] Rewrite `registry/tests/test_auth.py` — drop bearer-key scenarios, add `X-Session-Id`/`X-Agent-Id` header scenarios <!-- completed: -->
- [ ] Rewrite `registry/tests/test_registry_api.py` — replace api_key fixtures with session fixtures; update all `tenant_id` asserts to `session_id` <!-- completed: -->
- [ ] Rewrite `registry/tests/test_registry_store.py` — drop `create_api_key`/`list_api_keys`/`revoke_api_key` tests; add `create_session`/`list_sessions`/`delete_session` tests <!-- completed: -->
- [ ] Rewrite `registry/tests/test_executor.py` — rename `tenant_id` → `session_id`; add cross-session send rejection case asserting JSON-RPC error `-32001` <!-- completed: -->
- [ ] Rewrite `registry/tests/test_webui_api.py` — drop Auth0 mocks; use plain `X-Session-Id` header; assert `/ui/api/keys*` endpoints return 404 <!-- completed: -->
- [ ] Rewrite `registry/tests/test_webui_auth_migration.py` — if it was specifically about Auth0 migration, delete; otherwise rewrite to cover the 0002 upgrade path <!-- completed: -->
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
