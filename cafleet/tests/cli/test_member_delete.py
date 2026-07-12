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
    _member,
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
    the guard passes through (``director_member_id`` != ``MEMBER_ID``) without
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
            "director_member_id": 11,
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
        call_log.append(("deregister_member", member_id))
        return True

    monkeypatch.setattr(broker, "deregister_member", fake)
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
            "member",
            "delete",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(MEMBER_ID),
            *extra_args,
            "--json",
        ],
    )


def test_default_path__kills_pane_then_deregisters(
    runner,
    fleet_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    send_exit_recorder,
    kill_pane_recorder,
):
    """The single pane path: no flag, no ``/exit`` and no wait — ``kill_pane``
    (tolerating a missing pane) then deregister, exit 0. The header is the plain
    ``Member deleted.`` (no ``(--force)`` suffix) and the pane status is
    ``<pane> (killed)``."""
    monkeypatch.setattr(broker, "get_member", lambda *_a, **_kw: _member())

    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output

    assert send_exit_recorder == []
    assert kill_pane_recorder == [{"target_pane_id": PANE_ID, "ignore_missing": True}]
    assert deregister_recorder == [MEMBER_ID]

    names = [name for (name, *_) in call_log]
    assert names == [
        "kill_pane",
        "deregister_member",
    ]

    out = result.output
    assert "Member deleted." in out
    assert "(--force)" not in out
    assert str(MEMBER_ID) in out
    assert f"{PANE_ID} (killed)" in out


def test_default_path__json_output_pane_status_killed(
    runner,
    fleet_id,
    monkeypatch,
    deregister_recorder,
    kill_pane_recorder,
):
    monkeypatch.setattr(broker, "get_member", lambda *_a, **_kw: _member())

    result = _invoke_json(runner, fleet_id)
    assert result.exit_code == 0, result.output
    data = json.loads(result.stdout)
    assert data == {
        "member_id": MEMBER_ID,
        "pane_status": f"{PANE_ID} (killed)",
    }


def test_pane_already_gone__ignore_missing_swallow_yields_killed_exit_zero(
    runner,
    fleet_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    kill_pane_recorder,
):
    """An already-gone pane is tolerated: ``kill_pane`` is invoked with
    ``ignore_missing=True`` so the multiplexer swallows the missing-pane error,
    and the delete still deregisters and reports ``(killed)`` at exit 0 — the
    default path has no distinct 'already gone' outcome."""
    monkeypatch.setattr(broker, "get_member", lambda *_a, **_kw: _member())

    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output

    assert kill_pane_recorder == [{"target_pane_id": PANE_ID, "ignore_missing": True}]
    assert deregister_recorder == [MEMBER_ID]

    names = [name for (name, *_) in call_log]
    assert names == [
        "kill_pane",
        "deregister_member",
    ]

    out = result.output
    assert "Member deleted." in out
    assert "already gone" not in out
    assert f"{PANE_ID} (killed)" in out


def test_kill_pane_error__exits_one_with_backend_unreachable_wording(
    runner, fleet_id, monkeypatch, deregister_recorder
):
    """A ``MultiplexerError`` from ``kill_pane`` is a hard exit-1: the message
    names the pane, reports the backend 'server may be unreachable', and points
    at ``cafleet doctor``. The member is NOT deregistered."""
    monkeypatch.setattr(broker, "get_member", lambda *_a, **_kw: _member())

    def fake_kill_pane(self, **_kw):
        raise TmuxError("kill-pane failed: server exited unexpectedly")

    monkeypatch.setattr(TmuxMultiplexer, "kill_pane", fake_kill_pane, raising=False)
    result = _invoke(runner, fleet_id)

    assert result.exit_code == 1, (result.output, getattr(result, "stderr", ""))
    combined = (result.output or "") + (getattr(result, "stderr", "") or "")
    assert f"kill_pane failed for pane {PANE_ID}" in combined
    assert "server may be unreachable" in combined
    assert "cafleet doctor" in combined

    assert deregister_recorder == []


@pytest.mark.parametrize("removed_flag", ["--force", "-f"])
def test_force_flag_removed__unknown_option_exits_two(
    runner, fleet_id, deregister_recorder, removed_flag
):
    """``--force`` / ``-f`` no longer exist: Click rejects the unknown option
    with its default 'no such option' usage error (exit 2), before any member
    work — pinning the removal via the absence of the flag (`removal.md`)."""
    result = _invoke(runner, fleet_id, removed_flag)
    assert result.exit_code == 2, result.output
    assert "no such option" in result.output.lower()
    assert deregister_recorder == []


def test_pending_placement__skips_all_tmux(
    runner,
    fleet_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    send_exit_recorder,
    select_layout_recorder,
    kill_pane_recorder,
):
    """A pending placement (no pane id) is a pure registry soft-delete: exit 0,
    no tmux pane mutation whatsoever (no ``kill_pane`` / ``send_exit`` /
    ``select_layout``)."""
    monkeypatch.setattr(
        broker,
        "get_member",
        lambda *_a, **_kw: _member(placement=_placement(mux_pane_id=None)),
    )

    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output

    assert deregister_recorder == [MEMBER_ID]
    assert send_exit_recorder == []
    assert kill_pane_recorder == []
    assert select_layout_recorder == []

    names = [name for (name, *_) in call_log]
    assert names == ["deregister_member"]


def test_pending_placement__pending_pane_id_skips_kill_pane(
    runner, fleet_id, monkeypatch, deregister_recorder, kill_pane_recorder
):
    """Pending placements still deregister but skip the pane kill and report
    ``(pending — no pane)``."""
    monkeypatch.setattr(
        broker,
        "get_member",
        lambda *_a, **_kw: _member(placement=_placement(mux_pane_id=None)),
    )
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output
    assert deregister_recorder == [MEMBER_ID]
    assert kill_pane_recorder == []
    out = result.output
    assert "(pending" in out
    assert "no pane" in out


def test_authorization_boundary__missing_member_exits_one(
    runner, fleet_id, monkeypatch, deregister_recorder
):
    monkeypatch.setattr(broker, "get_member", lambda *_a, **_kw: None)
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert str(MEMBER_ID) in out
    assert "failed to fetch member" not in out
    assert f"Error: Member {MEMBER_ID} not found" in out
    assert deregister_recorder == []


def test_authorization_boundary__fetch_db_error_surfaces_failed_to_fetch_wording(
    runner, fleet_id, monkeypatch, deregister_recorder
):
    """Symmetric guard: real ``get_member`` failures surface as ClickException."""

    def boom(*_a, **_kw):
        raise RuntimeError("db connection lost")

    monkeypatch.setattr(broker, "get_member", boom)
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
):
    """A placementless target is a pure registry soft-delete: exit 0, no tmux
    pane mutation, ``pane_status`` ``(no placement)``."""
    monkeypatch.setattr(
        broker, "get_member", lambda *_a, **_kw: _member(placement=None)
    )
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output

    assert deregister_recorder == [MEMBER_ID]
    assert send_exit_recorder == []
    assert kill_pane_recorder == []
    names = [name for (name, *_) in call_log]
    assert names == ["deregister_member"]

    out = result.output
    assert "Member deleted." in out
    assert str(MEMBER_ID) in out
    assert "(no placement)" in out


def test_placementless__json_pane_status_no_placement(
    runner, fleet_id, monkeypatch, deregister_recorder
):
    monkeypatch.setattr(
        broker, "get_member", lambda *_a, **_kw: _member(placement=None)
    )
    result = _invoke_json(runner, fleet_id)
    assert result.exit_code == 0, result.output
    data = json.loads(result.stdout)
    assert data == {
        "member_id": MEMBER_ID,
        "pane_status": "(no placement)",
    }


def test_root_director_guard__broker_error_surfaces_verbatim(
    runner, fleet_id, monkeypatch
):
    """A broker deregister guard (the root-Director protection) reaches the
    operator verbatim (no ``deregister failed:`` wrapping), exit 1."""
    monkeypatch.setattr(
        broker, "get_member", lambda *_a, **_kw: _member(placement=None)
    )

    def guard(_member_id):
        raise click.ClickException("cannot deregister the root Director")

    monkeypatch.setattr(broker, "deregister_member", guard)
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert "Error: cannot deregister the root Director" in out
    assert "deregister failed" not in out


def _tmux_unavailable(self):
    raise TmuxError("tmux binary not found")


def test_tmux_relaxation__placementless_delete_succeeds_without_tmux(
    runner, fleet_id, monkeypatch, deregister_recorder
):
    """The tmux guard fires only on the pane-teardown paths — a placementless
    delete is a pure registry operation and works outside tmux."""
    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", _tmux_unavailable)
    monkeypatch.setattr(
        broker, "get_member", lambda *_a, **_kw: _member(placement=None)
    )
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
        "get_member",
        lambda *_a, **_kw: _member(placement=_placement(mux_pane_id=None)),
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
    monkeypatch.setattr(broker, "get_member", lambda *_a, **_kw: _member())
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 1, result.output
    assert "tmux binary not found" in (result.output or "")
    assert deregister_recorder == []


def test_root_director__rejected_before_any_tmux_pane_mutation(
    runner,
    fleet_id,
    monkeypatch,
    call_log,
    deregister_recorder,
    send_exit_recorder,
    kill_pane_recorder,
):
    """``member delete --member-id <root-director-id>`` is rejected with the
    root-Director error BEFORE any tmux pane mutation.

    The downstream ``broker.deregister_member`` root-guard fires only AFTER
    ``kill_pane``, so without the early guard a root delete would kill the
    Director's own pane before failing. This regression pins the early guard: no
    ``kill_pane`` / ``send_exit`` / deregister on the pane path.
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
            "director_member_id": MEMBER_ID,
        },
    )
    monkeypatch.setattr(broker, "get_member", lambda *_a, **_kw: _member())

    result = _invoke(runner, fleet_id)
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
    assert "deregister_member" not in names
