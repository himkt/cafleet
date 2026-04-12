"""Tests for hikyaku CLI member subgroup.

Covers: member create, member delete, member list, member capture.
All tmux interaction is mocked — no real tmux server required.
"""

import json
from unittest.mock import AsyncMock, patch

import pytest
from click.testing import CliRunner

from hikyaku_client.cli import cli

# ---------------------------------------------------------------------------
# Fixtures & constants
# ---------------------------------------------------------------------------

BROKER_URL = "http://localhost:8000"
API_KEY = "hky_a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4"
DIRECTOR_ID = "director-0000-0000-0000-000000000001"
MEMBER_ID = "member-0000-0000-0000-000000000002"

SAMPLE_REGISTER_RESULT = {
    "agent_id": MEMBER_ID,
    "api_key": API_KEY,
    "name": "Claude-B",
    "registered_at": "2026-04-12T10:15:00Z",
}

SAMPLE_PLACEMENT_VIEW = {
    "director_agent_id": DIRECTOR_ID,
    "tmux_session": "main",
    "tmux_window_id": "@3",
    "tmux_pane_id": "%7",
    "created_at": "2026-04-12T10:15:00Z",
}

SAMPLE_DIRECTOR_INFO = {
    "agent_id": DIRECTOR_ID,
    "name": "Director-A",
    "description": "Lead agent",
    "status": "active",
}

SAMPLE_MEMBER_INFO = {
    "agent_id": MEMBER_ID,
    "name": "Claude-B",
    "description": "Reviewer bot",
    "status": "active",
    "registered_at": "2026-04-12T10:15:00Z",
    "placement": SAMPLE_PLACEMENT_VIEW,
}


@pytest.fixture
def runner():
    return CliRunner()


def _auth_env():
    return {"HIKYAKU_URL": BROKER_URL, "HIKYAKU_API_KEY": API_KEY}


def _mock_tmux(monkeypatch):
    """Set up tmux env vars and mock the tmux module functions."""
    monkeypatch.setenv("TMUX", "/tmp/tmux-1000/default,12345,0")
    monkeypatch.setenv("TMUX_PANE", "%0")

    import hikyaku_client.tmux as tmux_mod

    monkeypatch.setattr(
        tmux_mod,
        "ensure_tmux_available",
        lambda: None,
    )
    monkeypatch.setattr(
        tmux_mod,
        "director_context",
        lambda: tmux_mod.DirectorContext(session="main", window_id="@3", pane_id="%0"),
    )
    monkeypatch.setattr(
        tmux_mod,
        "split_window",
        lambda **kw: "%7",
    )
    monkeypatch.setattr(
        tmux_mod,
        "select_layout",
        lambda **kw: None,
    )
    monkeypatch.setattr(
        tmux_mod,
        "send_exit",
        lambda **kw: None,
    )
    monkeypatch.setattr(
        tmux_mod,
        "capture_pane",
        lambda **kw: "captured line 1\ncaptured line 2\n",
    )
    return tmux_mod


# ---------------------------------------------------------------------------
# member create
# ---------------------------------------------------------------------------


class TestMemberCreate:
    def test_happy_path(self, runner, monkeypatch):
        """member create registers, splits, patches, rebalances — exit 0."""
        _mock_tmux(monkeypatch)

        register_mock = AsyncMock(return_value=SAMPLE_REGISTER_RESULT)
        patch_mock = AsyncMock(return_value=SAMPLE_PLACEMENT_VIEW)
        # list_agents used by _resolve_prompt for default prompt
        list_agents_mock = AsyncMock(return_value=SAMPLE_DIRECTOR_INFO)

        with (
            patch("hikyaku_client.cli.api.register_agent", register_mock),
            patch("hikyaku_client.cli.api.patch_placement", patch_mock),
            patch("hikyaku_client.cli.api.list_agents", list_agents_mock),
        ):
            result = runner.invoke(
                cli,
                [
                    "member",
                    "create",
                    "--agent-id",
                    DIRECTOR_ID,
                    "--name",
                    "Claude-B",
                    "--description",
                    "Reviewer bot",
                ],
                env=_auth_env(),
            )

        assert result.exit_code == 0
        assert MEMBER_ID in result.output
        register_mock.assert_called_once()
        patch_mock.assert_called_once()

    def test_rolls_back_on_split_failure(self, runner, monkeypatch):
        """If tmux split-window fails, registration is rolled back."""
        import hikyaku_client.tmux as tmux_mod

        _mock_tmux(monkeypatch)

        # Make split_window raise
        monkeypatch.setattr(
            tmux_mod,
            "split_window",
            lambda **kw: (_ for _ in ()).throw(tmux_mod.TmuxError("split failed")),
        )

        register_mock = AsyncMock(return_value=SAMPLE_REGISTER_RESULT)
        deregister_mock = AsyncMock(return_value=None)
        list_agents_mock = AsyncMock(return_value=SAMPLE_DIRECTOR_INFO)

        with (
            patch("hikyaku_client.cli.api.register_agent", register_mock),
            patch("hikyaku_client.cli.api.deregister_agent", deregister_mock),
            patch("hikyaku_client.cli.api.list_agents", list_agents_mock),
        ):
            result = runner.invoke(
                cli,
                [
                    "member",
                    "create",
                    "--agent-id",
                    DIRECTOR_ID,
                    "--name",
                    "Claude-B",
                    "--description",
                    "test",
                ],
                env=_auth_env(),
            )

        assert result.exit_code != 0
        # Rollback should have called deregister
        deregister_mock.assert_called_once()

    def test_default_prompt_when_no_trailing_args(self, runner, monkeypatch):
        """Without trailing args, default prompt is generated from director info."""
        _mock_tmux(monkeypatch)

        register_mock = AsyncMock(return_value=SAMPLE_REGISTER_RESULT)
        patch_mock = AsyncMock(return_value=SAMPLE_PLACEMENT_VIEW)
        list_agents_mock = AsyncMock(return_value=SAMPLE_DIRECTOR_INFO)

        with (
            patch("hikyaku_client.cli.api.register_agent", register_mock),
            patch("hikyaku_client.cli.api.patch_placement", patch_mock),
            patch("hikyaku_client.cli.api.list_agents", list_agents_mock),
        ):
            result = runner.invoke(
                cli,
                [
                    "member",
                    "create",
                    "--agent-id",
                    DIRECTOR_ID,
                    "--name",
                    "Claude-B",
                    "--description",
                    "test",
                ],
                env=_auth_env(),
            )

        assert result.exit_code == 0
        # list_agents called to fetch director info for default prompt
        list_agents_mock.assert_called_once()

    def test_trailing_positional_becomes_prompt(self, runner, monkeypatch):
        """Trailing args after -- become the prompt; no director lookup."""
        _mock_tmux(monkeypatch)

        register_mock = AsyncMock(return_value=SAMPLE_REGISTER_RESULT)
        patch_mock = AsyncMock(return_value=SAMPLE_PLACEMENT_VIEW)
        list_agents_mock = AsyncMock(return_value=SAMPLE_DIRECTOR_INFO)

        with (
            patch("hikyaku_client.cli.api.register_agent", register_mock),
            patch("hikyaku_client.cli.api.patch_placement", patch_mock),
            patch("hikyaku_client.cli.api.list_agents", list_agents_mock),
        ):
            result = runner.invoke(
                cli,
                [
                    "member",
                    "create",
                    "--agent-id",
                    DIRECTOR_ID,
                    "--name",
                    "Claude-B",
                    "--description",
                    "test",
                    "--",
                    "Review PR #42 and post feedback.",
                ],
                env=_auth_env(),
            )

        assert result.exit_code == 0
        # With trailing args, no need to look up director info
        list_agents_mock.assert_not_called()


# ---------------------------------------------------------------------------
# member delete
# ---------------------------------------------------------------------------


class TestMemberDelete:
    def test_idempotent_on_dead_pane(self, runner, monkeypatch):
        """Delete succeeds even when pane is already gone (ignore_missing=True)."""
        import hikyaku_client.tmux as tmux_mod

        _mock_tmux(monkeypatch)

        # send_exit returns silently (pane gone, but ignore_missing handles it)
        monkeypatch.setattr(tmux_mod, "send_exit", lambda **kw: None)

        list_agents_mock = AsyncMock(return_value=SAMPLE_MEMBER_INFO)
        deregister_mock = AsyncMock(return_value=None)

        with (
            patch("hikyaku_client.cli.api.list_agents", list_agents_mock),
            patch("hikyaku_client.cli.api.deregister_agent", deregister_mock),
        ):
            result = runner.invoke(
                cli,
                [
                    "member",
                    "delete",
                    "--agent-id",
                    DIRECTOR_ID,
                    "--member-id",
                    MEMBER_ID,
                ],
                env=_auth_env(),
            )

        assert result.exit_code == 0
        assert "Member deleted" in result.output
        deregister_mock.assert_called_once()

    def test_fail_fast_on_deregister_error(self, runner, monkeypatch):
        """If deregister fails, exit 1 — pane is preserved for retry."""
        _mock_tmux(monkeypatch)

        list_agents_mock = AsyncMock(return_value=SAMPLE_MEMBER_INFO)
        deregister_mock = AsyncMock(side_effect=Exception("403 Forbidden"))

        with (
            patch("hikyaku_client.cli.api.list_agents", list_agents_mock),
            patch("hikyaku_client.cli.api.deregister_agent", deregister_mock),
        ):
            result = runner.invoke(
                cli,
                [
                    "member",
                    "delete",
                    "--agent-id",
                    DIRECTOR_ID,
                    "--member-id",
                    MEMBER_ID,
                ],
                env=_auth_env(),
            )

        assert result.exit_code != 0
        assert "deregister failed" in (result.output + (result.stderr or "")).lower()


# ---------------------------------------------------------------------------
# member list
# ---------------------------------------------------------------------------


class TestMemberList:
    def test_json_output_shape(self, runner):
        """--json output is a JSON array with expected fields."""
        members = [
            {
                "agent_id": MEMBER_ID,
                "name": "Claude-B",
                "status": "active",
                "registered_at": "2026-04-12T10:15:00Z",
                "placement": SAMPLE_PLACEMENT_VIEW,
            }
        ]
        list_members_mock = AsyncMock(return_value=members)

        with patch("hikyaku_client.cli.api.list_members", list_members_mock):
            result = runner.invoke(
                cli,
                ["--json", "member", "list", "--agent-id", DIRECTOR_ID],
                env=_auth_env(),
            )

        assert result.exit_code == 0
        data = json.loads(result.output)
        assert isinstance(data, list)
        assert len(data) == 1
        item = data[0]
        assert "agent_id" in item
        assert "name" in item
        assert "status" in item
        assert "placement" in item
        p = item["placement"]
        assert "director_agent_id" in p
        assert "tmux_session" in p
        assert "tmux_window_id" in p
        assert "tmux_pane_id" in p
        assert "created_at" in p

    def test_renders_pending_pane_as_literal(self, runner):
        """Row with tmux_pane_id=None shows (pending) in text output and null in JSON."""
        pending_placement = {**SAMPLE_PLACEMENT_VIEW, "tmux_pane_id": None}
        members = [
            {
                "agent_id": MEMBER_ID,
                "name": "Claude-B",
                "status": "active",
                "registered_at": "2026-04-12T10:15:00Z",
                "placement": pending_placement,
            }
        ]
        list_members_mock = AsyncMock(return_value=members)

        # Text mode: (pending)
        with patch("hikyaku_client.cli.api.list_members", list_members_mock):
            result = runner.invoke(
                cli,
                ["member", "list", "--agent-id", DIRECTOR_ID],
                env=_auth_env(),
            )
        assert result.exit_code == 0
        assert "(pending)" in result.output

        # JSON mode: null
        with patch("hikyaku_client.cli.api.list_members", list_members_mock):
            result = runner.invoke(
                cli,
                ["--json", "member", "list", "--agent-id", DIRECTOR_ID],
                env=_auth_env(),
            )
        assert result.exit_code == 0
        data = json.loads(result.output)
        assert data[0]["placement"]["tmux_pane_id"] is None


# ---------------------------------------------------------------------------
# member capture
# ---------------------------------------------------------------------------


class TestMemberCapture:
    def test_happy_path(self, runner, monkeypatch):
        """Capture returns raw content on stdout (text mode)."""
        _mock_tmux(monkeypatch)

        list_agents_mock = AsyncMock(return_value=SAMPLE_MEMBER_INFO)

        with patch("hikyaku_client.cli.api.list_agents", list_agents_mock):
            result = runner.invoke(
                cli,
                [
                    "member",
                    "capture",
                    "--agent-id",
                    DIRECTOR_ID,
                    "--member-id",
                    MEMBER_ID,
                ],
                env=_auth_env(),
            )

        assert result.exit_code == 0
        assert "captured line 1" in result.output

    def test_cross_director_rejected(self, runner, monkeypatch):
        """Placement belongs to another Director — CLI exits 1."""
        _mock_tmux(monkeypatch)

        other_director_placement = {
            **SAMPLE_PLACEMENT_VIEW,
            "director_agent_id": "other-director-id",
        }
        target = {**SAMPLE_MEMBER_INFO, "placement": other_director_placement}
        list_agents_mock = AsyncMock(return_value=target)

        with patch("hikyaku_client.cli.api.list_agents", list_agents_mock):
            result = runner.invoke(
                cli,
                [
                    "member",
                    "capture",
                    "--agent-id",
                    DIRECTOR_ID,
                    "--member-id",
                    MEMBER_ID,
                ],
                env=_auth_env(),
            )

        assert result.exit_code != 0
        assert "not a member of your team" in (result.output + (result.stderr or ""))

    def test_pending_pane_rejected(self, runner, monkeypatch):
        """Placement has tmux_pane_id=None — CLI exits 1 with pending message."""
        _mock_tmux(monkeypatch)

        pending_placement = {**SAMPLE_PLACEMENT_VIEW, "tmux_pane_id": None}
        target = {**SAMPLE_MEMBER_INFO, "placement": pending_placement}
        list_agents_mock = AsyncMock(return_value=target)

        with patch("hikyaku_client.cli.api.list_agents", list_agents_mock):
            result = runner.invoke(
                cli,
                [
                    "member",
                    "capture",
                    "--agent-id",
                    DIRECTOR_ID,
                    "--member-id",
                    MEMBER_ID,
                ],
                env=_auth_env(),
            )

        assert result.exit_code != 0
        assert "pending placement" in (result.output + (result.stderr or ""))

    def test_no_placement_rejected(self, runner, monkeypatch):
        """Agent exists but has no placement row — CLI exits 1."""
        _mock_tmux(monkeypatch)

        target = {**SAMPLE_MEMBER_INFO, "placement": None}
        list_agents_mock = AsyncMock(return_value=target)

        with patch("hikyaku_client.cli.api.list_agents", list_agents_mock):
            result = runner.invoke(
                cli,
                [
                    "member",
                    "capture",
                    "--agent-id",
                    DIRECTOR_ID,
                    "--member-id",
                    MEMBER_ID,
                ],
                env=_auth_env(),
            )

        assert result.exit_code != 0
        assert "no placement row" in (result.output + (result.stderr or ""))

    def test_json_shape(self, runner, monkeypatch):
        """--json output has exactly {member_agent_id, pane_id, lines, content}."""
        _mock_tmux(monkeypatch)

        list_agents_mock = AsyncMock(return_value=SAMPLE_MEMBER_INFO)

        with patch("hikyaku_client.cli.api.list_agents", list_agents_mock):
            result = runner.invoke(
                cli,
                [
                    "--json",
                    "member",
                    "capture",
                    "--agent-id",
                    DIRECTOR_ID,
                    "--member-id",
                    MEMBER_ID,
                ],
                env=_auth_env(),
            )

        assert result.exit_code == 0
        data = json.loads(result.output)
        assert set(data.keys()) == {"member_agent_id", "pane_id", "lines", "content"}
        assert data["member_agent_id"] == MEMBER_ID
        assert data["pane_id"] == "%7"
        assert data["lines"] == 80
        assert "captured line 1" in data["content"]
