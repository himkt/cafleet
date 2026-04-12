"""Tests for auth.py — session-based request dependencies.

Design doc 0000015 Step 5 replaces bearer-token authentication with two
session-based FastAPI dependencies:

  - ``get_session_from_agent_id(request, store)``
    Reads ``X-Agent-Id`` header, looks up ``agents.session_id``, returns
    ``(agent_id, session_id)``. Raises 400 when the header is missing,
    404 when the agent does not exist.

  - ``get_session_from_header(request, store)``
    Reads ``X-Session-Id`` header, verifies existence in ``sessions``
    table, returns ``session_id``. Raises 400 when the header is missing,
    404 when the session does not exist.

All Auth0 / bearer / API-key concepts are removed. There is no
``get_authenticated_agent``, ``get_registration_tenant``,
``Auth0Verifier``, ``verify_auth0_user``, or ``get_user_id``.
"""

import uuid
from datetime import UTC, datetime

import pytest
from fastapi import HTTPException
from unittest.mock import MagicMock
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from hikyaku.auth import get_session_from_agent_id, get_session_from_header
from hikyaku.db.models import Session


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


async def _create_test_session(
    db_sessionmaker: async_sessionmaker[AsyncSession],
    *,
    session_id: str | None = None,
    label: str | None = None,
) -> str:
    """Seed a session row directly via the DB sessionmaker."""
    if session_id is None:
        session_id = str(uuid.uuid4())
    created_at = datetime.now(UTC).isoformat()

    async with db_sessionmaker() as session:
        async with session.begin():
            session.add(
                Session(
                    session_id=session_id,
                    label=label,
                    created_at=created_at,
                )
            )
    return session_id


def _make_request(
    *,
    x_agent_id: str | None = None,
    x_session_id: str | None = None,
) -> MagicMock:
    """Construct a mock Request with optional headers."""
    request = MagicMock()
    headers: dict[str, str] = {}
    if x_agent_id is not None:
        headers["x-agent-id"] = x_agent_id
    if x_session_id is not None:
        headers["x-session-id"] = x_session_id
    request.headers = headers
    return request


# ===========================================================================
# get_session_from_agent_id tests
# ===========================================================================


class TestGetSessionFromAgentId:
    """``get_session_from_agent_id`` — resolves agent → session.

    Used by JSON-RPC ``POST /`` and REST endpoints that identify the
    caller by agent (e.g., ``DELETE /agents/{id}``).
    """

    async def test_valid_agent_returns_tuple(self, store, db_sessionmaker):
        """Valid X-Agent-Id returns ``(agent_id, session_id)``."""
        session_id = await _create_test_session(db_sessionmaker)
        agent = await store.create_agent("a", "d", None, session_id=session_id)
        request = _make_request(x_agent_id=agent["agent_id"])

        result = await get_session_from_agent_id(request, store)

        assert result == (agent["agent_id"], session_id)

    async def test_missing_header_raises_400(self, store):
        """Missing X-Agent-Id header raises HTTPException(400)."""
        request = _make_request()

        with pytest.raises(HTTPException) as exc_info:
            await get_session_from_agent_id(request, store)

        assert exc_info.value.status_code == 400

    async def test_nonexistent_agent_raises_404(self, store):
        """X-Agent-Id that doesn't exist in the agents table raises 404."""
        fake_id = str(uuid.uuid4())
        request = _make_request(x_agent_id=fake_id)

        with pytest.raises(HTTPException) as exc_info:
            await get_session_from_agent_id(request, store)

        assert exc_info.value.status_code == 404

    async def test_deregistered_agent_still_resolves(self, store, db_sessionmaker):
        """A deregistered agent's row still has session_id; lookup must work.

        The agent record persists (soft delete) and its session_id FK is
        still valid. Some read-only endpoints (e.g., GET inbox for a
        deregistered agent) rely on this.
        """
        session_id = await _create_test_session(db_sessionmaker)
        agent = await store.create_agent("a", "d", None, session_id=session_id)
        await store.deregister_agent(agent["agent_id"])

        request = _make_request(x_agent_id=agent["agent_id"])
        result = await get_session_from_agent_id(request, store)

        assert result == (agent["agent_id"], session_id)

    async def test_returns_correct_session_for_agent(self, store, db_sessionmaker):
        """Two agents in different sessions resolve to their own session."""
        session_a = await _create_test_session(db_sessionmaker)
        session_b = await _create_test_session(db_sessionmaker)
        agent_a = await store.create_agent("a", "d", None, session_id=session_a)
        agent_b = await store.create_agent("b", "d", None, session_id=session_b)

        req_a = _make_request(x_agent_id=agent_a["agent_id"])
        req_b = _make_request(x_agent_id=agent_b["agent_id"])

        result_a = await get_session_from_agent_id(req_a, store)
        result_b = await get_session_from_agent_id(req_b, store)

        assert result_a[1] == session_a
        assert result_b[1] == session_b


# ===========================================================================
# get_session_from_header tests
# ===========================================================================


class TestGetSessionFromHeader:
    """``get_session_from_header`` — resolves X-Session-Id → session_id.

    Used by REST endpoints that do not identify a caller agent
    (e.g., ``GET /agents``, ``GET /agents/{id}``).
    """

    async def test_valid_session_returns_session_id(self, store, db_sessionmaker):
        """Valid X-Session-Id header returns the session_id string."""
        session_id = await _create_test_session(db_sessionmaker)
        request = _make_request(x_session_id=session_id)

        result = await get_session_from_header(request, store)

        assert result == session_id

    async def test_missing_header_raises_400(self, store):
        """Missing X-Session-Id header raises HTTPException(400)."""
        request = _make_request()

        with pytest.raises(HTTPException) as exc_info:
            await get_session_from_header(request, store)

        assert exc_info.value.status_code == 400

    async def test_nonexistent_session_raises_404(self, store):
        """X-Session-Id that doesn't exist in the sessions table raises 404."""
        request = _make_request(x_session_id="nonexistent-session-id")

        with pytest.raises(HTTPException) as exc_info:
            await get_session_from_header(request, store)

        assert exc_info.value.status_code == 404

    async def test_empty_session_id_raises_400(self, store):
        """An empty X-Session-Id header value is treated as missing → 400."""
        request = _make_request(x_session_id="")

        with pytest.raises(HTTPException) as exc_info:
            await get_session_from_header(request, store)

        assert exc_info.value.status_code == 400


# ===========================================================================
# Deleted functions — verify they no longer exist
# ===========================================================================


class TestDeletedAuthFunctions:
    """Verify that Auth0 and bearer-related functions are removed.

    Design doc 0000015 Step 5: these are deleted entirely.
    """

    def test_auth0_verifier_removed(self):
        """Auth0Verifier class must not exist in auth module."""
        from hikyaku import auth

        assert not hasattr(auth, "Auth0Verifier")

    def test_verify_auth0_user_removed(self):
        """verify_auth0_user must not exist in auth module."""
        from hikyaku import auth

        assert not hasattr(auth, "verify_auth0_user")

    def test_get_user_id_removed(self):
        """get_user_id must not exist in auth module."""
        from hikyaku import auth

        assert not hasattr(auth, "get_user_id")

    def test_get_authenticated_agent_removed(self):
        """get_authenticated_agent (bearer-based) must not exist."""
        from hikyaku import auth

        assert not hasattr(auth, "get_authenticated_agent")

    def test_get_registration_tenant_removed(self):
        """get_registration_tenant must not exist."""
        from hikyaku import auth

        assert not hasattr(auth, "get_registration_tenant")


class TestDeletedConfigFields:
    """Verify that Auth0 config fields are removed from Settings.

    Design doc 0000015 Step 5: delete auth0_domain, auth0_client_id,
    auth0_audience from config.py.
    """

    def test_auth0_domain_removed(self):
        from hikyaku.config import Settings

        assert "auth0_domain" not in Settings.model_fields

    def test_auth0_client_id_removed(self):
        from hikyaku.config import Settings

        assert "auth0_client_id" not in Settings.model_fields

    def test_auth0_audience_removed(self):
        from hikyaku.config import Settings

        assert "auth0_audience" not in Settings.model_fields


class TestPyJWTRemoved:
    """Verify that PyJWT is no longer importable from auth.py.

    Design doc 0000015 Step 5: remove PyJWT from dependencies.
    The auth module must not import ``jwt`` at the top level.
    """

    def test_auth_module_does_not_import_jwt(self):
        """The auth module should not have ``jwt`` in its namespace."""
        import importlib

        # Force reimport to catch top-level imports
        auth = importlib.import_module("hikyaku.auth")
        assert not hasattr(auth, "jwt"), (
            "auth.py still imports 'jwt' — PyJWT should be removed"
        )

    def test_auth_module_does_not_import_httpbearer(self):
        """The auth module should not import HTTPBearer (used only for Auth0)."""
        from hikyaku import auth

        assert not hasattr(auth, "HTTPBearer")
        assert not hasattr(auth, "HTTPAuthorizationCredentials")
