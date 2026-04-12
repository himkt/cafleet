"""Tests for CLI register command — CAFLEET_SESSION_ID env var required.

Design doc 0000015 Step 8: CAFLEET_API_KEY → CAFLEET_SESSION_ID,
register gains _require_session_id entry check, session_id sent in
POST body and X-Session-Id header (same code path as every other command).

Covers: register uses CAFLEET_SESSION_ID env var, error message when
missing (mentioning 'cafleet session create'), session_id
passed to api.register_agent, X-Session-Id header on outgoing requests.
"""

import json
from unittest.mock import AsyncMock, patch

import pytest
from click.testing import CliRunner

from cafleet.cli import cli


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def runner():
    """Provide a click.testing.CliRunner."""
    return CliRunner()


# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

BROKER_URL = "http://127.0.0.1:8000"
SESSION_ID = "550e8400-e29b-41d4-a716-446655440001"
AGENT_ID = "550e8400-e29b-41d4-a716-446655440000"

SAMPLE_AGENT = {
    "agent_id": AGENT_ID,
    "name": "test-agent",
    "description": "A test agent",
    "status": "active",
}


# ===========================================================================
# Register uses CAFLEET_SESSION_ID env var
# ===========================================================================


class TestRegisterUsesSessionId:
    """Tests for register command using CAFLEET_SESSION_ID env var.

    The register command reads session_id from ctx.obj['session_id'],
    which is populated from the CAFLEET_SESSION_ID environment variable.
    """

    def test_session_id_passed_to_register(self, runner):
        """Register with CAFLEET_SESSION_ID env var passes it to api.register_agent."""
        mock = AsyncMock(return_value=SAMPLE_AGENT)
        with patch("cafleet.cli.api.register_agent", mock):
            result = runner.invoke(
                cli,
                [
                    "register",
                    "--name",
                    "test-agent",
                    "--description",
                    "A test agent",
                ],
                env={"CAFLEET_URL": BROKER_URL, "CAFLEET_SESSION_ID": SESSION_ID},
            )

        assert result.exit_code == 0
        call_kwargs = mock.call_args
        assert call_kwargs is not None
        # session_id should be passed to register_agent
        all_args = list(call_kwargs.args) + list(call_kwargs.kwargs.values())
        assert (
            SESSION_ID in all_args or call_kwargs.kwargs.get("session_id") == SESSION_ID
        )

    def test_session_id_via_env_var(self, runner):
        """Register uses CAFLEET_SESSION_ID env var for session scoping."""
        mock = AsyncMock(return_value=SAMPLE_AGENT)
        with patch("cafleet.cli.api.register_agent", mock):
            result = runner.invoke(
                cli,
                ["register", "--name", "test-agent", "--description", "test"],
                env={
                    "CAFLEET_URL": BROKER_URL,
                    "CAFLEET_SESSION_ID": SESSION_ID,
                },
            )

        assert result.exit_code == 0

    def test_register_success_with_session_id(self, runner):
        """Register succeeds and shows output when CAFLEET_SESSION_ID is set."""
        mock = AsyncMock(return_value=SAMPLE_AGENT)
        with patch("cafleet.cli.api.register_agent", mock):
            result = runner.invoke(
                cli,
                [
                    "register",
                    "--name",
                    "test-agent",
                    "--description",
                    "A test agent",
                ],
                env={"CAFLEET_URL": BROKER_URL, "CAFLEET_SESSION_ID": SESSION_ID},
            )

        assert result.exit_code == 0
        assert AGENT_ID in result.output

    def test_register_json_output_with_session_id(self, runner):
        """Register with --json outputs valid JSON when CAFLEET_SESSION_ID is set."""
        mock = AsyncMock(return_value=SAMPLE_AGENT)
        with patch("cafleet.cli.api.register_agent", mock):
            result = runner.invoke(
                cli,
                [
                    "--json",
                    "register",
                    "--name",
                    "test-agent",
                    "--description",
                    "A test agent",
                ],
                env={"CAFLEET_URL": BROKER_URL, "CAFLEET_SESSION_ID": SESSION_ID},
            )

        assert result.exit_code == 0
        data = json.loads(result.output)
        assert data["agent_id"] == AGENT_ID


# ===========================================================================
# Missing CAFLEET_SESSION_ID env var error
# ===========================================================================


class TestRegisterMissingSessionId:
    """Tests for register command when CAFLEET_SESSION_ID is missing.

    Register must validate that CAFLEET_SESSION_ID env var is set and show
    a specific error message if not. Design doc specifies the exact message:
    "Error: CAFLEET_SESSION_ID environment variable is required. Create a
    session with 'cafleet session create'."
    """

    def test_missing_session_id_shows_error(self, runner):
        """Register without CAFLEET_SESSION_ID prints error and exits non-zero."""
        result = runner.invoke(
            cli,
            [
                "register",
                "--name",
                "test-agent",
                "--description",
                "A test agent",
            ],
            env={"CAFLEET_URL": BROKER_URL},
        )

        assert result.exit_code != 0

    def test_missing_session_id_error_message(self, runner):
        """Error message mentions CAFLEET_SESSION_ID environment variable."""
        result = runner.invoke(
            cli,
            [
                "register",
                "--name",
                "test-agent",
                "--description",
                "A test agent",
            ],
            env={"CAFLEET_URL": BROKER_URL},
        )

        output = result.output + (result.stderr or "")
        assert "CAFLEET_SESSION_ID" in output

    def test_missing_session_id_mentions_session_create(self, runner):
        """Error message mentions 'cafleet session create'.

        Design doc: error message should say "Create a session with
        'cafleet session create'." (replaces old "Create an
        API key at the CAFleet WebUI.")
        """
        result = runner.invoke(
            cli,
            [
                "register",
                "--name",
                "test-agent",
                "--description",
                "A test agent",
            ],
            env={"CAFLEET_URL": BROKER_URL},
        )

        output = result.output + (result.stderr or "")
        assert "cafleet session create" in output

    def test_missing_session_id_does_not_call_api(self, runner):
        """Register without CAFLEET_SESSION_ID does not make any API call."""
        mock = AsyncMock(return_value=SAMPLE_AGENT)
        with patch("cafleet.cli.api.register_agent", mock):
            runner.invoke(
                cli,
                [
                    "register",
                    "--name",
                    "test-agent",
                    "--description",
                    "A test agent",
                ],
                env={"CAFLEET_URL": BROKER_URL},
            )

        mock.assert_not_called()


# ===========================================================================
# api.register_agent sends X-Session-Id header (not Authorization: Bearer)
# ===========================================================================


class TestApiRegisterAgentSessionHeader:
    """Tests for api.register_agent sending X-Session-Id header.

    Design doc: api.py sends X-Session-Id: <value> instead of
    Authorization: Bearer <value>.
    """

    @pytest.mark.asyncio
    async def test_sends_x_session_id_header(self):
        """register_agent sends X-Session-Id header (not Authorization: Bearer)."""
        from cafleet.broker_client import register_agent

        with patch("cafleet.broker_client.httpx.AsyncClient") as mock_client_cls:
            mock_client = AsyncMock()
            mock_response = AsyncMock()
            mock_response.json.return_value = SAMPLE_AGENT
            mock_response.raise_for_status = AsyncMock()
            mock_client.post.return_value = mock_response
            mock_client.__aenter__ = AsyncMock(return_value=mock_client)
            mock_client.__aexit__ = AsyncMock(return_value=False)
            mock_client_cls.return_value = mock_client

            await register_agent(
                BROKER_URL, "test-agent", "A test agent", session_id=SESSION_ID
            )

            call_kwargs = mock_client.post.call_args
            headers = call_kwargs.kwargs.get("headers", {})
            assert "X-Session-Id" in headers, (
                "register_agent should send X-Session-Id header"
            )
            assert headers["X-Session-Id"] == SESSION_ID
            assert "Authorization" not in headers, (
                "register_agent should NOT send Authorization: Bearer header"
            )

    @pytest.mark.asyncio
    async def test_session_id_in_post_body(self):
        """register_agent includes session_id in the POST body.

        Design doc: api.register_agent adds session_id to the POST body
        (matching the new POST /api/v1/agents contract).
        """
        from cafleet.broker_client import register_agent

        with patch("cafleet.broker_client.httpx.AsyncClient") as mock_client_cls:
            mock_client = AsyncMock()
            mock_response = AsyncMock()
            mock_response.json.return_value = SAMPLE_AGENT
            mock_response.raise_for_status = AsyncMock()
            mock_client.post.return_value = mock_response
            mock_client.__aenter__ = AsyncMock(return_value=mock_client)
            mock_client.__aexit__ = AsyncMock(return_value=False)
            mock_client_cls.return_value = mock_client

            await register_agent(
                BROKER_URL, "test-agent", "A test agent", session_id=SESSION_ID
            )

            call_kwargs = mock_client.post.call_args
            body = call_kwargs.kwargs.get("json", {})
            assert body.get("session_id") == SESSION_ID, (
                "register_agent POST body should include session_id"
            )

    @pytest.mark.asyncio
    async def test_session_id_is_required_parameter(self):
        """register_agent requires session_id (not optional)."""
        from cafleet.broker_client import register_agent

        with pytest.raises(TypeError):
            await register_agent(BROKER_URL, "test-agent", "A test agent")


# ===========================================================================
# Deleted patterns — verify no api_key / Authorization: Bearer references
# ===========================================================================


class TestDeletedApiKeyPatterns:
    """Verify that api_key and Authorization: Bearer are removed from api.py.

    Design doc 0000015 Step 8: api_key parameters renamed to session_id,
    Authorization: Bearer replaced with X-Session-Id.
    """

    def test_no_authorization_bearer_in_api(self):
        """api.py should not contain 'Authorization: Bearer' strings."""
        import inspect
        from cafleet import broker_client as api_module

        source = inspect.getsource(api_module)
        assert "Authorization" not in source, (
            "api.py should not reference 'Authorization' — "
            "all requests should use X-Session-Id header"
        )
        assert "Bearer" not in source, (
            "api.py should not reference 'Bearer' — "
            "all requests should use X-Session-Id header"
        )

    def test_no_api_key_parameter_in_api(self):
        """api.py functions should not have api_key parameter."""
        import inspect
        from cafleet import broker_client as api_module

        source = inspect.getsource(api_module)
        # api_key should be renamed to session_id
        assert "api_key" not in source, (
            "api.py should not have api_key parameters — "
            "they should be renamed to session_id"
        )


# ===========================================================================
# No SQLAlchemy dependency in client
# ===========================================================================


class TestNoSQLAlchemyDependency:
    """Verify client has no SQLAlchemy dependency.

    Design doc: client/ remains HTTP-only. SQLAlchemy/aiosqlite stay
    in registry/ only.
    """

    def test_no_sqlalchemy_import(self):
        """Client modules should not import sqlalchemy."""
        import sys

        # Ensure cafleet is loaded
        import cafleet.cli  # noqa: F401
        import cafleet.broker_client  # noqa: F401

        client_modules = [name for name in sys.modules if name.startswith("cafleet")]
        for mod_name in client_modules:
            mod = sys.modules[mod_name]
            if mod is not None:
                assert not hasattr(mod, "sqlalchemy"), (
                    f"{mod_name} should not import sqlalchemy"
                )
