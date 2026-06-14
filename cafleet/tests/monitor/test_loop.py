"""Tests for ``monitor_tick`` — one full scan pass (§4, §5).

The global ``_silence_real_tmux_subprocess`` fixture already stubs the tmux
``_run`` subprocess; each test additionally stubs ``list_pane_ids`` (the
per-tick liveness query) and captures the per-role keystrokes —
``send_poll_trigger`` for the Director, ``send_wake_trigger`` for the dedicated
monitoring member. After design 0000090 the loop pings ONLY those two roles;
ordinary members are not enrolled and ``send_resume_trigger`` no longer exists.
"""

import os
from datetime import UTC, datetime, timedelta

import pytest

from cafleet import broker
from cafleet.db.models import Fleet
from cafleet.monitor.loop import CONTINUE, STOP, monitor_tick
from tests.broker._helpers import _create_fleet, _register_agent

_NOW = datetime(2026, 6, 13, 12, 0, 0, tzinfo=UTC)


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


def _placement(fleet: dict, pane_id: str) -> dict:
    return {
        "director_agent_id": fleet["director"]["agent_id"],
        "tmux_session": "main",
        "tmux_window_id": "@3",
        "tmux_pane_id": pane_id,
        "coding_agent": "claude",
    }


def _register_member(fleet: dict, name: str, pane_id: str) -> int:
    """Register an ordinary (non-enrolled) member under the fleet's Director."""
    return _register_agent(
        fleet["fleet_id"], name=name, placement=_placement(fleet, pane_id)
    )["agent_id"]


def _register_monitoring_member(fleet: dict, name: str, pane_id: str) -> int:
    """Register the dedicated monitoring member (``kind=monitoring-member``)."""
    return broker.register_agent(
        fleet_id=fleet["fleet_id"],
        name=name,
        description="monitoring member",
        placement=_placement(fleet, pane_id),
        kind="monitoring-member",
    )["agent_id"]


def _stub_tmux(monkeypatch, live_panes):
    """Stub pane liveness; capture poll-trigger (Director) and wake-trigger
    (monitoring member) keystrokes into separate lists (§4)."""
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.list_pane_ids",
        lambda self: set(live_panes),
        raising=False,
    )
    polls = []
    wakes = []

    def fake_poll(self, *, target_pane_id, fleet_id, agent_id):
        polls.append((target_pane_id, fleet_id, agent_id))
        return True

    def fake_wake(self, *, target_pane_id, fleet_id, agent_id):
        wakes.append((target_pane_id, fleet_id, agent_id))
        return True

    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.send_poll_trigger", fake_poll
    )
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.send_wake_trigger",
        fake_wake,
        raising=False,
    )
    return polls, wakes


def test_monitor_tick__poll_to_director_wake_to_monitor_skips_ordinary(
    capsys, monkeypatch
):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    watcher = _register_monitoring_member(fleet, "watcher", "%7")  # enrolled → wake
    alice = _register_member(fleet, "alice", "%9")  # ordinary, alive but NOT enrolled

    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )

    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # §4: the Director receives a bare poll; the monitoring member a wake nudge
    assert {agent_id for _, _, agent_id in polls} == {director_id}
    assert {agent_id for _, _, agent_id in wakes} == {watcher}
    assert ("%0", sid, director_id) in polls
    assert ("%7", sid, watcher) in wakes

    # the ordinary member is never enrolled, so it is never pinged by either path
    assert alice not in {agent_id for _, _, agent_id in polls}
    assert alice not in {agent_id for _, _, agent_id in wakes}
    assert broker.get_monitor_config(sid, alice) is None

    # record_ping advanced both enrolled roles; the ordinary member has no config
    assert (
        broker.get_monitor_config(sid, director_id)["last_ping_at"] == _NOW.isoformat()
    )
    assert broker.get_monitor_config(sid, watcher)["last_ping_at"] == _NOW.isoformat()

    # the heartbeat was written for this tick
    assert broker.read_monitor_runtime(sid)["last_tick_at"] == _NOW.isoformat()

    # one stdout ping-log line per dispatched ping; the ordinary member is absent
    out = capsys.readouterr().out
    assert f"ping agent {director_id} (Director)" in out
    assert f"ping agent {watcher} (watcher)" in out
    assert "alice" not in out


def test_monitor_tick__stop_on_soft_deleted_fleet(broker_session, monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    broker.claim_monitor_runtime(sid, os.getpid(), 5, _NOW.isoformat())

    # soft-delete WITHOUT removing the runtime row, so the heartbeat still
    # succeeds and the deleted_at branch is what triggers STOP
    with broker_session() as s:
        s.get(Fleet, sid).deleted_at = _NOW.isoformat()
        s.commit()

    polls, wakes = _stub_tmux(monkeypatch, set())
    assert monitor_tick(sid, _NOW) is STOP
    assert polls == []
    assert wakes == []


def test_monitor_tick__stop_on_missing_fleet(monkeypatch):
    # the fleet row vanished under a live monitor: the ownership-checked
    # heartbeat passes, then get_fleet returns None → STOP (defensive §5 branch)
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    broker.claim_monitor_runtime(sid, os.getpid(), 5, _NOW.isoformat())
    monkeypatch.setattr("cafleet.broker.get_fleet", lambda fleet_id: None)

    polls, wakes = _stub_tmux(monkeypatch, set())
    assert monitor_tick(sid, _NOW) is STOP
    assert polls == []
    assert wakes == []


def test_monitor_tick__stop_when_ownership_lost(monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    # the slot is owned by a different pid → this tick's ownership-checked
    # heartbeat matches zero rows and the loser self-terminates without pinging
    broker.claim_monitor_runtime(sid, os.getpid() + 1, 5, _NOW.isoformat())

    polls, wakes = _stub_tmux(monkeypatch, {"%0"})
    assert monitor_tick(sid, _NOW) is STOP
    assert polls == []
    assert wakes == []
