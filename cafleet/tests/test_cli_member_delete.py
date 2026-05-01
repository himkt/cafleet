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


@pytest.fixture
def call_log() -> list[tuple]:
    return []


@pytest.fixture(autouse=True)
def _stub_tmux_entrypoints(monkeypatch):
    monkeypatch.setattr(tmux, "ensure_tmux_available", lambda: None)
    monkeypatch.setattr(tmux, "director_context", lambda: _DIRECTOR_CTX)
    monkeypatch.setattr(tmux, "send_exit", lambda **_: None)
    monkeypatch.setattr(tmux, "select_layout", lambda **_: None)
    monkeypatch.setattr(tmux, "kill_pane", lambda **_: None, raising=False)
    monkeypatch.setattr(tmux, "wait_for_pane_gone", lambda **_: True, raising=False)
    monkeypatch.setattr(tmux, "pane_exists", lambda **_: False, raising=False)
    monkeypatch.setattr(tmux, "capture_pane", lambda **_: "", raising=False)


@pytest.fixture
def deregister_recorder(monkeypatch, call_log):
    calls: list[str] = []

    def fake(member_id):
        calls.append(member_id)
        call_log.append(("deregister_agent", member_id))
        return True

    monkeypatch.setattr(broker, "deregister_agent", fake)
    return calls


@pytest.fixture
def send_exit_recorder(monkeypatch, call_log):
    calls: list[dict] = []

    def fake(**kwargs):
        calls.append(kwargs)
        call_log.append(("send_exit", kwargs))

    monkeypatch.setattr(tmux, "send_exit", fake)
    return calls


@pytest.fixture
def select_layout_recorder(monkeypatch, call_log):
    calls: list[dict] = []

    def fake(**kwargs):
        calls.append(kwargs)
        call_log.append(("select_layout", kwargs))

    monkeypatch.setattr(tmux, "select_layout", fake)
    return calls


@pytest.fixture
def wait_for_pane_gone_recorder(monkeypatch, call_log):
    """Recording stub with mutable return / side-effect via .state."""
    state: dict = {"return_value": True, "side_effect": None}
    calls: list[dict] = []

    def fake(**kwargs):
        calls.append(kwargs)
        call_log.append(("wait_for_pane_gone", kwargs))
        if state["side_effect"] is not None:
            raise state["side_effect"]
        return state["return_value"]

    monkeypatch.setattr(tmux, "wait_for_pane_gone", fake, raising=False)
    fake.calls = calls
    fake.state = state
    return fake


@pytest.fixture
def capture_pane_recorder(monkeypatch, call_log):
    state: dict = {"return_value": "", "side_effect": None}
    calls: list[dict] = []

    def fake(**kwargs):
        calls.append(kwargs)
        call_log.append(("capture_pane", kwargs))
        if state["side_effect"] is not None:
            raise state["side_effect"]
        return state["return_value"]

    monkeypatch.setattr(tmux, "capture_pane", fake, raising=False)
    fake.calls = calls
    fake.state = state
    return fake


@pytest.fixture
def kill_pane_recorder(monkeypatch, call_log):
    calls: list[dict] = []

    def fake(**kwargs):
        calls.append(kwargs)
        call_log.append(("kill_pane", kwargs))

    monkeypatch.setattr(tmux, "kill_pane", fake, raising=False)
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


def _invoke_json(runner, session_id, *extra_args):
    return runner.invoke(
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
            *extra_args,
        ],
    )


def test_happy_path__call_ordering_send_exit_then_wait_then_deregister_then_layout(
    runner,
    session_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    send_exit_recorder,
    select_layout_recorder,
    wait_for_pane_gone_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
    wait_for_pane_gone_recorder.state["return_value"] = True

    result = _invoke(runner, session_id)
    assert result.exit_code == 0, result.output

    names = [name for (name, *_) in call_log]
    assert names == [
        "send_exit",
        "wait_for_pane_gone",
        "deregister_agent",
        "select_layout",
    ]

    assert send_exit_recorder == [{"target_pane_id": PANE_ID, "ignore_missing": True}]
    assert deregister_recorder == [MEMBER_ID]

    out = result.output
    assert "Member deleted." in out
    assert MEMBER_ID in out
    assert f"{PANE_ID} (closed)" in out


def test_happy_path__json_output_returns_agent_id_and_pane_status(
    runner,
    session_id,
    monkeypatch,
    deregister_recorder,
    wait_for_pane_gone_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
    wait_for_pane_gone_recorder.state["return_value"] = True

    result = _invoke_json(runner, session_id)
    assert result.exit_code == 0, result.output
    data = json.loads(result.stdout)
    assert data == {
        "agent_id": MEMBER_ID,
        "pane_status": f"{PANE_ID} (closed)",
    }


def test_pane_already_gone__pane_already_gone_first_poll_yields_happy_path(
    runner,
    session_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    send_exit_recorder,
    select_layout_recorder,
    wait_for_pane_gone_recorder,
    capture_pane_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
    wait_for_pane_gone_recorder.state["return_value"] = True

    result = _invoke(runner, session_id)
    assert result.exit_code == 0, result.output

    assert capture_pane_recorder.calls == []

    names = [name for (name, *_) in call_log]
    assert "capture_pane" not in names
    assert names == [
        "send_exit",
        "wait_for_pane_gone",
        "deregister_agent",
        "select_layout",
    ]

    assert deregister_recorder == [MEMBER_ID]

    out = result.output
    assert "Member deleted." in out
    assert "already gone" not in out
    assert f"{PANE_ID} (closed)" in out


def test_timeout__timeout_exits_two_with_tail_and_recovery_hint(
    runner,
    session_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    send_exit_recorder,
    select_layout_recorder,
    wait_for_pane_gone_recorder,
    capture_pane_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
    wait_for_pane_gone_recorder.state["return_value"] = False
    capture_pane_recorder.state["return_value"] = "STUCK_BUFFER_TAIL"

    result = _invoke(runner, session_id)
    assert result.exit_code == 2, (result.output, getattr(result, "stderr", ""))

    combined = (result.output or "") + (getattr(result, "stderr", "") or "")
    assert f"pane {PANE_ID} did not close within 15.0s" in combined
    assert "STUCK_BUFFER_TAIL" in combined
    assert "cafleet member capture" in combined
    assert "cafleet member send-input" in combined
    assert "--force" in combined

    assert deregister_recorder == []
    assert select_layout_recorder == []

    names = [name for (name, *_) in call_log]
    assert "deregister_agent" not in names
    assert "select_layout" not in names
    assert names == [
        "send_exit",
        "wait_for_pane_gone",
        "capture_pane",
    ]


def test_timeout__timeout_json_output_pane_status(
    runner,
    session_id,
    monkeypatch,
    deregister_recorder,
    wait_for_pane_gone_recorder,
    capture_pane_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
    wait_for_pane_gone_recorder.state["return_value"] = False
    capture_pane_recorder.state["return_value"] = "STUCK_BUFFER_TAIL"

    result = _invoke_json(runner, session_id)
    assert result.exit_code == 2, result.output
    data = json.loads(result.stdout)
    assert data == {
        "agent_id": MEMBER_ID,
        "pane_status": f"{PANE_ID} (timeout)",
    }


def test_timeout__capture_failure_still_exits_two(
    runner,
    session_id,
    monkeypatch,
    deregister_recorder,
    send_exit_recorder,
    wait_for_pane_gone_recorder,
    capture_pane_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
    wait_for_pane_gone_recorder.state["return_value"] = False
    capture_pane_recorder.state["side_effect"] = TmuxError(
        "capture-pane failed: pane is dead"
    )

    result = _invoke(runner, session_id)
    assert result.exit_code == 2, (result.output, getattr(result, "stderr", ""))

    combined = (result.output or "") + (getattr(result, "stderr", "") or "")
    assert "Warning: capture_pane failed during timeout handling" in combined
    assert "timeout error and recovery hint still print" in combined
    assert f"pane {PANE_ID} did not close within 15.0s" in combined
    assert "cafleet member capture" in combined
    assert "cafleet member send-input" in combined
    assert "--force" in combined

    assert deregister_recorder == []


def test_force__force_kills_pane_then_deregisters(
    runner,
    session_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    send_exit_recorder,
    select_layout_recorder,
    kill_pane_recorder,
    wait_for_pane_gone_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())

    result = _invoke(runner, session_id, "--force")
    assert result.exit_code == 0, result.output

    assert send_exit_recorder == []
    assert wait_for_pane_gone_recorder.calls == []

    assert kill_pane_recorder == [{"target_pane_id": PANE_ID, "ignore_missing": True}]
    assert deregister_recorder == [MEMBER_ID]

    names = [name for (name, *_) in call_log]
    assert names == [
        "kill_pane",
        "deregister_agent",
        "select_layout",
    ]

    out = result.output
    assert "Member deleted (--force)." in out
    assert f"{PANE_ID} (killed)" in out


def test_force__force_short_flag_works(
    runner,
    session_id,
    monkeypatch,
    deregister_recorder,
    send_exit_recorder,
    kill_pane_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())

    result = _invoke(runner, session_id, "-f")
    assert result.exit_code == 0, result.output
    assert send_exit_recorder == []
    assert kill_pane_recorder == [{"target_pane_id": PANE_ID, "ignore_missing": True}]


def test_force__force_json_output_pane_status_killed(
    runner,
    session_id,
    monkeypatch,
    deregister_recorder,
    kill_pane_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())

    result = _invoke_json(runner, session_id, "--force")
    assert result.exit_code == 0, result.output
    data = json.loads(result.stdout)
    assert data == {
        "agent_id": MEMBER_ID,
        "pane_status": f"{PANE_ID} (killed)",
    }


def test_pending_placement_force__force_with_pending_placement_skips_all_tmux(
    runner,
    session_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    send_exit_recorder,
    select_layout_recorder,
    kill_pane_recorder,
    wait_for_pane_gone_recorder,
):
    monkeypatch.setattr(
        broker,
        "get_agent",
        lambda *_a, **_kw: _agent(placement=_placement(tmux_pane_id=None)),
    )

    result = _invoke(runner, session_id, "--force")
    assert result.exit_code == 0, result.output

    assert deregister_recorder == [MEMBER_ID]
    assert send_exit_recorder == []
    assert kill_pane_recorder == []
    assert select_layout_recorder == []
    assert wait_for_pane_gone_recorder.calls == []

    names = [name for (name, *_) in call_log]
    assert names == ["deregister_agent"]


def test_authorization_boundary__missing_agent_exits_one(
    runner, session_id, monkeypatch, deregister_recorder
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: None)
    result = _invoke(runner, session_id)
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert MEMBER_ID in out
    assert "failed to fetch member" not in out
    assert f"Error: Agent {MEMBER_ID} not found" in out
    assert deregister_recorder == []


def test_authorization_boundary__fetch_db_error_surfaces_failed_to_fetch_wording(
    runner, session_id, monkeypatch, deregister_recorder
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


def test_authorization_boundary__placement_none_exits_one_with_deregister_hint(
    runner, session_id, monkeypatch, deregister_recorder
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent(placement=None))
    result = _invoke(runner, session_id)
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert f"agent {MEMBER_ID}" in out
    assert "has no placement" in out
    assert "cafleet agent deregister" in out
    assert deregister_recorder == []


def test_authorization_boundary__cross_director_same_session_is_rejected(
    runner, session_id, monkeypatch, deregister_recorder, send_exit_recorder
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


def test_pending_placement__pending_pane_id_skips_send_exit(
    runner, session_id, monkeypatch, deregister_recorder, send_exit_recorder
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


def test_tmux_error_on_send_exit__send_exit_failure_now_exits_one_with_recovery_wording(
    runner, session_id, monkeypatch, deregister_recorder
):
    """Under design 0000032 §3, send_exit TmuxError is a hard exit-1.

    The old behavior was warning-and-continue with `tmux kill-pane -t <pane>`
    in the warning text. The new wording points operators at `cafleet doctor`
    and `--force` instead, with no raw tmux command exposed.
    """
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())

    def fake_send_exit(**_kw):
        raise TmuxError("send-keys failed: pane is dead")

    monkeypatch.setattr(tmux, "send_exit", fake_send_exit)
    result = _invoke(runner, session_id)

    assert result.exit_code == 1, (result.output, getattr(result, "stderr", ""))
    combined = (result.output or "") + (getattr(result, "stderr", "") or "")
    assert "send_exit failed" in combined
    assert "tmux server may be unreachable" in combined
    assert "cafleet doctor" in combined
    assert "--force" in combined
    assert "tmux kill-pane" not in combined

    assert deregister_recorder == []
