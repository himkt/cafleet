"""CLI tests for ``cafleet member delete`` (flat fleet-isolation model)."""

import json

import click
import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli
from cafleet.multiplexer import MultiplexerContext as DirectorContext
from cafleet.multiplexer.tmux import TmuxError, TmuxMultiplexer
from tests.cli._member_helpers import (
    MEMBER_ID,
    PANE_ID,
    _agent,
    _placement,
)

_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")


@pytest.fixture
def fleet_id():
    return 100


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture
def call_log() -> list[tuple]:
    return []


@pytest.fixture(autouse=True)
def _stub_get_fleet(monkeypatch):
    """``member_delete``'s early root-guard calls ``broker.get_fleet``; stub it so
    the guard passes through (``director_agent_id`` != ``MEMBER_ID``) without
    touching the real DB. The root-delete-guard tests override this to make the
    guard fire."""
    monkeypatch.setattr(
        broker,
        "get_fleet",
        lambda sid: {
            "fleet_id": sid,
            "name": None,
            "created_at": "2026-05-05T00:00:00+00:00",
            "deleted_at": None,
            "director_agent_id": 11,
        },
    )


@pytest.fixture(autouse=True)
def _stub_tmux_entrypoints(monkeypatch):
    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", lambda self: None)
    monkeypatch.setattr(
        TmuxMultiplexer, "context_discovery", lambda self: _DIRECTOR_CTX
    )
    monkeypatch.setattr(TmuxMultiplexer, "send_exit", lambda self, **_: None)
    monkeypatch.setattr(TmuxMultiplexer, "select_layout", lambda self, **_: None)
    monkeypatch.setattr(
        TmuxMultiplexer, "kill_pane", lambda self, **_: None, raising=False
    )
    monkeypatch.setattr(
        TmuxMultiplexer, "wait_for_pane_gone", lambda self, **_: True, raising=False
    )
    monkeypatch.setattr(
        TmuxMultiplexer, "pane_exists", lambda self, **_: False, raising=False
    )
    monkeypatch.setattr(
        TmuxMultiplexer, "capture_pane", lambda self, **_: "", raising=False
    )


def _make_kwargs_recorder(
    monkeypatch,
    call_log,
    module,
    func_name,
    *,
    stateful: bool = False,
    default_return=None,
):
    """Record kwargs-only calls; with ``stateful=True`` expose ``.calls`` + ``.state``."""
    calls: list[dict] = []
    state: dict = {"return_value": default_return, "side_effect": None}

    def fake(self, **kwargs):
        calls.append(kwargs)
        call_log.append((func_name, kwargs))
        if stateful and state["side_effect"] is not None:
            raise state["side_effect"]
        return state["return_value"] if stateful else None

    monkeypatch.setattr(module, func_name, fake, raising=False)
    if stateful:
        fake.calls = calls
        fake.state = state
        return fake
    return calls


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
    return _make_kwargs_recorder(monkeypatch, call_log, TmuxMultiplexer, "send_exit")


@pytest.fixture
def select_layout_recorder(monkeypatch, call_log):
    return _make_kwargs_recorder(
        monkeypatch, call_log, TmuxMultiplexer, "select_layout"
    )


@pytest.fixture
def wait_for_pane_gone_recorder(monkeypatch, call_log):
    return _make_kwargs_recorder(
        monkeypatch,
        call_log,
        TmuxMultiplexer,
        "wait_for_pane_gone",
        stateful=True,
        default_return=True,
    )


@pytest.fixture
def capture_pane_recorder(monkeypatch, call_log):
    return _make_kwargs_recorder(
        monkeypatch,
        call_log,
        TmuxMultiplexer,
        "capture_pane",
        stateful=True,
        default_return="",
    )


@pytest.fixture
def kill_pane_recorder(monkeypatch, call_log):
    return _make_kwargs_recorder(monkeypatch, call_log, TmuxMultiplexer, "kill_pane")


def _invoke(runner, fleet_id, *extra_args):
    return runner.invoke(
        cli,
        [
            "member",
            "delete",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(MEMBER_ID),
            *extra_args,
        ],
    )


def _invoke_json(runner, fleet_id, *extra_args):
    return runner.invoke(
        cli,
        [
            "--json",
            "member",
            "delete",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(MEMBER_ID),
            *extra_args,
        ],
    )


def test_happy_path__call_ordering_send_exit_then_wait_then_deregister(
    runner,
    fleet_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    send_exit_recorder,
    wait_for_pane_gone_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
    wait_for_pane_gone_recorder.state["return_value"] = True

    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output

    names = [name for (name, *_) in call_log]
    assert names == [
        "send_exit",
        "wait_for_pane_gone",
        "deregister_agent",
    ]

    assert send_exit_recorder == [{"target_pane_id": PANE_ID, "ignore_missing": True}]
    assert deregister_recorder == [MEMBER_ID]

    out = result.output
    assert "Member deleted." in out
    assert str(MEMBER_ID) in out
    assert f"{PANE_ID} (closed)" in out


def test_happy_path__json_output_returns_agent_id_and_pane_status(
    runner,
    fleet_id,
    monkeypatch,
    deregister_recorder,
    wait_for_pane_gone_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
    wait_for_pane_gone_recorder.state["return_value"] = True

    result = _invoke_json(runner, fleet_id)
    assert result.exit_code == 0, result.output
    data = json.loads(result.stdout)
    assert data == {
        "agent_id": MEMBER_ID,
        "pane_status": f"{PANE_ID} (closed)",
    }


def test_pane_already_gone__pane_already_gone_first_poll_yields_happy_path(
    runner,
    fleet_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    send_exit_recorder,
    wait_for_pane_gone_recorder,
    capture_pane_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
    wait_for_pane_gone_recorder.state["return_value"] = True

    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output

    assert capture_pane_recorder.calls == []

    names = [name for (name, *_) in call_log]
    assert "capture_pane" not in names
    assert names == [
        "send_exit",
        "wait_for_pane_gone",
        "deregister_agent",
    ]

    assert deregister_recorder == [MEMBER_ID]

    out = result.output
    assert "Member deleted." in out
    assert "already gone" not in out
    assert f"{PANE_ID} (closed)" in out


def test_timeout__timeout_exits_two_with_tail_and_recovery_hint(
    runner,
    fleet_id,
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

    result = _invoke(runner, fleet_id)
    assert result.exit_code == 2, (result.output, getattr(result, "stderr", ""))

    combined = (result.output or "") + (getattr(result, "stderr", "") or "")
    assert f"pane {PANE_ID} did not close within 15.0s" in combined
    assert "STUCK_BUFFER_TAIL" in combined
    assert "cafleet member capture" in combined
    assert "cafleet member delete" in combined
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
    fleet_id,
    monkeypatch,
    deregister_recorder,
    wait_for_pane_gone_recorder,
    capture_pane_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
    wait_for_pane_gone_recorder.state["return_value"] = False
    capture_pane_recorder.state["return_value"] = "STUCK_BUFFER_TAIL"

    result = _invoke_json(runner, fleet_id)
    assert result.exit_code == 2, result.output
    data = json.loads(result.stdout)
    assert data == {
        "agent_id": MEMBER_ID,
        "pane_status": f"{PANE_ID} (timeout)",
    }


def test_timeout__capture_failure_still_exits_two(
    runner,
    fleet_id,
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

    result = _invoke(runner, fleet_id)
    assert result.exit_code == 2, (result.output, getattr(result, "stderr", ""))

    combined = (result.output or "") + (getattr(result, "stderr", "") or "")
    assert "Warning: capture_pane failed during timeout handling" in combined
    assert "timeout error and recovery hint still print" in combined
    assert f"pane {PANE_ID} did not close within 15.0s" in combined
    assert "cafleet member capture" in combined
    assert "cafleet member delete" in combined
    assert "--force" in combined

    assert deregister_recorder == []


def test_force__force_kills_pane_then_deregisters(
    runner,
    fleet_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    send_exit_recorder,
    kill_pane_recorder,
    wait_for_pane_gone_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())

    result = _invoke(runner, fleet_id, "--force")
    assert result.exit_code == 0, result.output

    assert send_exit_recorder == []
    assert wait_for_pane_gone_recorder.calls == []

    assert kill_pane_recorder == [{"target_pane_id": PANE_ID, "ignore_missing": True}]
    assert deregister_recorder == [MEMBER_ID]

    names = [name for (name, *_) in call_log]
    assert names == [
        "kill_pane",
        "deregister_agent",
    ]

    out = result.output
    assert "Member deleted (--force)." in out
    assert f"{PANE_ID} (killed)" in out


def test_force__force_short_flag_works(
    runner,
    fleet_id,
    monkeypatch,
    deregister_recorder,
    send_exit_recorder,
    kill_pane_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())

    result = _invoke(runner, fleet_id, "-f")
    assert result.exit_code == 0, result.output
    assert send_exit_recorder == []
    assert kill_pane_recorder == [{"target_pane_id": PANE_ID, "ignore_missing": True}]


def test_force__force_json_output_pane_status_killed(
    runner,
    fleet_id,
    monkeypatch,
    deregister_recorder,
    kill_pane_recorder,
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())

    result = _invoke_json(runner, fleet_id, "--force")
    assert result.exit_code == 0, result.output
    data = json.loads(result.stdout)
    assert data == {
        "agent_id": MEMBER_ID,
        "pane_status": f"{PANE_ID} (killed)",
    }


def test_pending_placement_force__force_with_pending_placement_skips_all_tmux(
    runner,
    fleet_id,
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
        lambda *_a, **_kw: _agent(placement=_placement(mux_pane_id=None)),
    )

    result = _invoke(runner, fleet_id, "--force")
    assert result.exit_code == 0, result.output

    assert deregister_recorder == [MEMBER_ID]
    assert send_exit_recorder == []
    assert kill_pane_recorder == []
    assert select_layout_recorder == []
    assert wait_for_pane_gone_recorder.calls == []

    names = [name for (name, *_) in call_log]
    assert names == ["deregister_agent"]


def test_authorization_boundary__missing_agent_exits_one(
    runner, fleet_id, monkeypatch, deregister_recorder
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: None)
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert str(MEMBER_ID) in out
    assert "failed to fetch member" not in out
    assert f"Error: Agent {MEMBER_ID} not found" in out
    assert deregister_recorder == []


def test_authorization_boundary__fetch_db_error_surfaces_failed_to_fetch_wording(
    runner, fleet_id, monkeypatch, deregister_recorder
):
    """Symmetric guard: real ``get_agent`` failures surface as ClickException."""

    def boom(*_a, **_kw):
        raise RuntimeError("db connection lost")

    monkeypatch.setattr(broker, "get_agent", boom)
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 1
    out = result.output or ""
    assert "failed to fetch member" in out
    assert "db connection lost" in out
    assert deregister_recorder == []


def test_placementless__soft_delete_succeeds_without_pane_mutation(
    runner,
    fleet_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    send_exit_recorder,
    kill_pane_recorder,
    wait_for_pane_gone_recorder,
):
    """A placementless target is a pure registry soft-delete: exit 0, no tmux
    pane mutation, ``pane_status`` ``(no placement)``."""
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent(placement=None))
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output

    assert deregister_recorder == [MEMBER_ID]
    assert send_exit_recorder == []
    assert kill_pane_recorder == []
    assert wait_for_pane_gone_recorder.calls == []
    names = [name for (name, *_) in call_log]
    assert names == ["deregister_agent"]

    out = result.output
    assert "Member deleted." in out
    assert str(MEMBER_ID) in out
    assert "(no placement)" in out


def test_placementless__json_pane_status_no_placement(
    runner, fleet_id, monkeypatch, deregister_recorder
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent(placement=None))
    result = _invoke_json(runner, fleet_id)
    assert result.exit_code == 0, result.output
    data = json.loads(result.stdout)
    assert data == {
        "agent_id": MEMBER_ID,
        "pane_status": "(no placement)",
    }


def test_administrator_guard__broker_error_surfaces_verbatim(
    runner, fleet_id, monkeypatch
):
    """The broker's Administrator guard reaches the operator verbatim (no
    ``deregister failed:`` wrapping), exit 1."""
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent(placement=None))

    def guard(_member_id):
        raise click.ClickException("Administrator cannot be deregistered")

    monkeypatch.setattr(broker, "deregister_agent", guard)
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert "Error: Administrator cannot be deregistered" in out
    assert "deregister failed" not in out


def _tmux_unavailable(self):
    raise TmuxError("tmux binary not found")


def test_tmux_relaxation__placementless_delete_succeeds_without_tmux(
    runner, fleet_id, monkeypatch, deregister_recorder
):
    """The tmux guard fires only on the pane-teardown paths — a placementless
    delete is a pure registry operation and works outside tmux."""
    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", _tmux_unavailable)
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent(placement=None))
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output
    assert deregister_recorder == [MEMBER_ID]
    assert "(no placement)" in result.output


def test_tmux_relaxation__pending_placement_delete_succeeds_without_tmux(
    runner, fleet_id, monkeypatch, deregister_recorder
):
    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", _tmux_unavailable)
    monkeypatch.setattr(
        broker,
        "get_agent",
        lambda *_a, **_kw: _agent(placement=_placement(mux_pane_id=None)),
    )
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output
    assert deregister_recorder == [MEMBER_ID]


def test_tmux_relaxation__live_pane_delete_still_requires_tmux(
    runner, fleet_id, monkeypatch, deregister_recorder
):
    """A live-pane teardown still needs tmux: unavailable tmux fails the
    delete before any deregister."""
    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", _tmux_unavailable)
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 1, result.output
    assert "tmux binary not found" in (result.output or "")
    assert deregister_recorder == []


def test_pending_placement__pending_pane_id_skips_send_exit(
    runner, fleet_id, monkeypatch, deregister_recorder, send_exit_recorder
):
    """Pending placements still deregister but skip the pane ``/exit``."""
    monkeypatch.setattr(
        broker,
        "get_agent",
        lambda *_a, **_kw: _agent(placement=_placement(mux_pane_id=None)),
    )
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output
    assert deregister_recorder == [MEMBER_ID]
    assert send_exit_recorder == []
    out = result.output
    assert "(pending" in out
    assert "no pane" in out


def test_tmux_error_on_send_exit__send_exit_failure_now_exits_one_with_recovery_wording(
    runner, fleet_id, monkeypatch, deregister_recorder
):
    """send_exit TmuxError is a hard exit-1.

    The wording points operators at `cafleet doctor` and `--force`, with no
    raw tmux command exposed.
    """
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())

    def fake_send_exit(self, **_kw):
        raise TmuxError("send-keys failed: pane is dead")

    monkeypatch.setattr(TmuxMultiplexer, "send_exit", fake_send_exit)
    result = _invoke(runner, fleet_id)

    assert result.exit_code == 1, (result.output, getattr(result, "stderr", ""))
    combined = (result.output or "") + (getattr(result, "stderr", "") or "")
    assert "send_exit failed" in combined
    assert "tmux server may be unreachable" in combined
    assert "cafleet doctor" in combined
    assert "--force" in combined
    assert "tmux kill-pane" not in combined

    assert deregister_recorder == []


def test_tmux_error_on_wait_for_pane_gone__exits_one_with_backend_name(
    runner, fleet_id, monkeypatch, deregister_recorder
):
    """A MultiplexerError while waiting for the pane to close is a hard exit-1,
    surfaced with the resolved backend name (``{mux.name}`` = tmux in the test
    env) and the target pane id. The member is not deregistered."""
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())
    monkeypatch.setattr(TmuxMultiplexer, "send_exit", lambda self, **_kw: None)

    def fake_wait(self, **_kw):
        raise TmuxError("list-panes failed: server exited unexpectedly")

    monkeypatch.setattr(TmuxMultiplexer, "wait_for_pane_gone", fake_wait, raising=False)
    result = _invoke(runner, fleet_id)

    assert result.exit_code == 1, (result.output, getattr(result, "stderr", ""))
    combined = (result.output or "") + (getattr(result, "stderr", "") or "")
    assert "tmux call failed while waiting" in combined
    assert f"pane {PANE_ID}" in combined

    assert deregister_recorder == []


@pytest.mark.parametrize("extra_args", [[], ["--force"]])
def test_root_director__rejected_before_any_tmux_pane_mutation(
    runner,
    fleet_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    send_exit_recorder,
    kill_pane_recorder,
    extra_args,
):
    """``member delete --member-id <root-director-id>`` is rejected with the
    root-Director error BEFORE any tmux pane mutation.

    The downstream ``broker.deregister_agent`` root-guard fires only AFTER
    ``send_exit`` / ``kill_pane``, so without the early guard a root delete
    would inject ``/exit`` into (or kill) the Director's own pane before
    failing. This regression pins the early guard: no ``send_exit`` /
    ``kill_pane`` / deregister on either the default or ``--force`` path.
    """
    # The targeted member IS the fleet root Director.
    monkeypatch.setattr(
        broker,
        "get_fleet",
        lambda _sid: {
            "fleet_id": fleet_id,
            "name": None,
            "created_at": "2026-05-05T00:00:00+00:00",
            "deleted_at": None,
            "director_agent_id": MEMBER_ID,
        },
    )
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: _agent())

    result = _invoke(runner, fleet_id, *extra_args)
    assert result.exit_code != 0, result.output
    out = result.output or ""
    assert "root Director" in out
    assert "fleet delete" in out

    # No pane mutation, no deregister — the guard short-circuits first.
    assert send_exit_recorder == []
    assert kill_pane_recorder == []
    assert deregister_recorder == []
    names = [name for (name, *_) in call_log]
    assert "send_exit" not in names
    assert "kill_pane" not in names
    assert "deregister_agent" not in names
