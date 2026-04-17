"""CLI tests for ``cafleet member delete`` (cross-Director guard regression)."""

import json
import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker, tmux
from cafleet.cli import cli
from cafleet.tmux import DirectorContext, TmuxError

DIRECTOR_ID = "11111111-1111-1111-1111-111111111111"
MEMBER_ID = "22222222-2222-2222-2222-222222222222"
OTHER_DIRECTOR_ID = "33333333-3333-3333-3333-333333333333"
PANE_ID = "%7"
MEMBER_NAME = "Claude-B"

_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")

_UNSET: object = object()


def _placement(
    *,
    director_agent_id: str = DIRECTOR_ID,
    tmux_pane_id: str | None = PANE_ID,
    coding_agent: str = "claude",
) -> dict:
    return {
        "director_agent_id": director_agent_id,
        "tmux_session": "main",
        "tmux_window_id": "@3",
        "tmux_pane_id": tmux_pane_id,
        "coding_agent": coding_agent,
        "created_at": "2026-04-16T08:00:00+00:00",
    }


def _agent(
    *,
    agent_id: str = MEMBER_ID,
    name: str = MEMBER_NAME,
    placement: dict | None | object = _UNSET,
) -> dict:
    resolved_placement = _placement() if placement is _UNSET else placement
    return {
        "agent_id": agent_id,
        "name": name,
        "description": "Test member",
        "status": "active",
        "registered_at": "2026-04-16T08:00:00+00:00",
        "kind": "user",
        "placement": resolved_placement,
    }


@pytest.fixture
def session_id():
    return str(uuid.uuid4())


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture(autouse=True)
def _stub_tmux_entrypoints(monkeypatch):
    monkeypatch.setattr(tmux, "ensure_tmux_available", lambda: None)
    monkeypatch.setattr(tmux, "director_context", lambda: _DIRECTOR_CTX)
    monkeypatch.setattr(tmux, "send_exit", lambda **_: None)
    monkeypatch.setattr(tmux, "select_layout", lambda **_: None)


@pytest.fixture
def deregister_recorder(monkeypatch):
    calls: list[str] = []

    def fake(member_id):
        calls.append(member_id)
        return True

    monkeypatch.setattr(broker, "deregister_agent", fake)
    return calls


@pytest.fixture
def send_exit_recorder(monkeypatch):
    calls: list[dict] = []

    def fake(**kwargs):
        calls.append(kwargs)

    monkeypatch.setattr(tmux, "send_exit", fake)
    return calls


def _invoke(runner, session_id, *extra_args):
    return runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "member",
            "delete",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
            *extra_args,
        ],
    )


class TestHappyPath:
    def test_deregisters_and_sends_exit_to_pane(
        self, runner, session_id, monkeypatch, deregister_recorder, send_exit_recorder
    ):
        monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
        result = _invoke(runner, session_id)
        assert result.exit_code == 0, result.output
        assert deregister_recorder == [MEMBER_ID]
        assert send_exit_recorder == [
            {"target_pane_id": PANE_ID, "ignore_missing": True}
        ]
        out = result.output
        assert "Member deleted." in out
        assert MEMBER_ID in out
        assert f"{PANE_ID} (closed)" in out

    def test_json_output_returns_agent_id_and_pane_status(
        self, runner, session_id, monkeypatch, deregister_recorder
    ):
        monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "--json",
                "member",
                "delete",
                "--agent-id",
                DIRECTOR_ID,
                "--member-id",
                MEMBER_ID,
            ],
        )
        assert result.exit_code == 0, result.output
        data = json.loads(result.output)
        assert data == {
            "agent_id": MEMBER_ID,
            "pane_status": f"{PANE_ID} (closed)",
        }


class TestAuthorizationBoundary:
    def test_missing_agent_exits_one(
        self, runner, session_id, monkeypatch, deregister_recorder
    ):
        monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: None)
        result = _invoke(runner, session_id)
        assert result.exit_code == 1, result.output
        out = result.output or ""
        assert MEMBER_ID in out
        assert "failed to fetch member" not in out
        assert f"Error: Agent {MEMBER_ID} not found" in out
        assert deregister_recorder == []

    def test_fetch_db_error_surfaces_failed_to_fetch_wording(
        self, runner, session_id, monkeypatch, deregister_recorder
    ):
        """Symmetric guard: real ``get_agent`` failures keep the wrapper wording."""

        def boom(*_a, **_kw):
            raise RuntimeError("db connection lost")

        monkeypatch.setattr(broker, "get_agent", boom)
        result = _invoke(runner, session_id)
        assert result.exit_code == 1
        out = result.output or ""
        assert "failed to fetch member" in out
        assert "db connection lost" in out
        assert deregister_recorder == []

    def test_placement_none_exits_one_with_deregister_hint(
        self, runner, session_id, monkeypatch, deregister_recorder
    ):
        monkeypatch.setattr(
            broker, "get_agent", lambda *_a, **_kw: _agent(placement=None)
        )
        result = _invoke(runner, session_id)
        assert result.exit_code == 1, result.output
        out = result.output or ""
        assert f"agent {MEMBER_ID}" in out
        assert "has no placement" in out
        assert "cafleet deregister" in out
        assert deregister_recorder == []

    def test_cross_director_same_session_is_rejected(
        self, runner, session_id, monkeypatch, deregister_recorder, send_exit_recorder
    ):
        """Regression guard for the cross-Director auth gap in ``member_delete``."""
        monkeypatch.setattr(
            broker,
            "get_agent",
            lambda *_a, **_kw: _agent(
                placement=_placement(director_agent_id=OTHER_DIRECTOR_ID)
            ),
        )
        result = _invoke(runner, session_id)
        assert result.exit_code == 1, result.output
        out = result.output or ""
        assert f"agent {MEMBER_ID}" in out
        assert "is not a member of your team" in out
        assert OTHER_DIRECTOR_ID in out
        assert deregister_recorder == []
        assert send_exit_recorder == []


class TestPendingPlacement:
    def test_pending_pane_id_skips_send_exit(
        self, runner, session_id, monkeypatch, deregister_recorder, send_exit_recorder
    ):
        """Pending placements still deregister but skip the pane ``/exit``."""
        monkeypatch.setattr(
            broker,
            "get_agent",
            lambda *_a, **_kw: _agent(placement=_placement(tmux_pane_id=None)),
        )
        result = _invoke(runner, session_id)
        assert result.exit_code == 0, result.output
        assert deregister_recorder == [MEMBER_ID]
        assert send_exit_recorder == []
        out = result.output
        assert "(pending" in out
        assert "no pane" in out


class TestTmuxErrorOnSendExit:
    def test_send_exit_failure_is_surfaced_as_warning(
        self, runner, session_id, monkeypatch, deregister_recorder
    ):
        """``send_exit`` failure after deregister must warn and still exit 0."""
        monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())

        def fake_send_exit(**_kw):
            raise TmuxError("send-keys failed: pane is dead")

        monkeypatch.setattr(tmux, "send_exit", fake_send_exit)
        result = _invoke(runner, session_id)
        assert result.exit_code == 0, result.output
        assert deregister_recorder == [MEMBER_ID]
        out = result.output
        assert "Warning: send_exit failed" in out
        assert f"tmux kill-pane -t {PANE_ID}" in out
        assert f"{PANE_ID} (send_exit failed)" in out
