"""Tests for ``monitor_tick`` — one full scan pass (§5).

The global ``_silence_real_tmux_subprocess`` fixture already stubs the tmux
``_run`` subprocess; each test additionally stubs ``list_pane_ids`` (the
per-tick liveness query) and captures ``send_poll_trigger`` keystrokes.
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


def _register_member(fleet: dict, name: str, pane_id: str) -> int:
    return _register_agent(
        fleet["fleet_id"],
        name=name,
        placement={
            "director_agent_id": fleet["director"]["agent_id"],
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": pane_id,
            "coding_agent": "claude",
        },
    )["agent_id"]


def _stub_tmux(monkeypatch, live_panes):
    """Stub the per-tick pane-liveness query and capture poll-trigger keystrokes."""
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.list_pane_ids",
        lambda self: set(live_panes),
        raising=False,
    )
    pings = []

    def fake_poll(self, *, target_pane_id, fleet_id, agent_id):
        pings.append((target_pane_id, fleet_id, agent_id))
        return True

    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.send_poll_trigger", fake_poll
    )
    return pings


def test_monitor_tick__pings_exactly_due_agents_records_and_heartbeats(monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    alice = _register_member(fleet, "alice", "%7")  # pending, alive  → ping
    bob = _register_member(fleet, "bob", "%9")  # no pending, alive  → skip
    carol = _register_member(fleet, "carol", "%99")  # pending, dead → skip

    broker.send_message(sid, director_id, alice, "do x")
    broker.send_message(sid, director_id, carol, "do y")
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )

    pings = _stub_tmux(monkeypatch, {"%0", "%7", "%9"})  # carol's %99 is dead

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    assert {agent_id for _, _, agent_id in pings} == {director_id, alice}
    assert ("%0", sid, director_id) in pings
    assert ("%7", sid, alice) in pings

    # record_ping advanced only the pinged agents
    assert (
        broker.get_monitor_config(sid, director_id)["last_ping_at"] == _NOW.isoformat()
    )
    assert broker.get_monitor_config(sid, alice)["last_ping_at"] == _NOW.isoformat()
    assert broker.get_monitor_config(sid, bob)["last_ping_at"] is None
    assert broker.get_monitor_config(sid, carol)["last_ping_at"] is None

    # the heartbeat was written for this tick
    assert broker.read_monitor_runtime(sid)["last_tick_at"] == _NOW.isoformat()


def test_monitor_tick__stop_on_soft_deleted_fleet(broker_session, monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    broker.claim_monitor_runtime(sid, os.getpid(), 5, _NOW.isoformat())

    # soft-delete WITHOUT removing the runtime row, so the heartbeat still
    # succeeds and the deleted_at branch is what triggers STOP
    with broker_session() as s:
        s.get(Fleet, sid).deleted_at = _NOW.isoformat()
        s.commit()

    pings = _stub_tmux(monkeypatch, set())
    assert monitor_tick(sid, _NOW) is STOP
    assert pings == []


def test_monitor_tick__stop_on_missing_fleet(monkeypatch):
    # the fleet row vanished under a live monitor: the ownership-checked
    # heartbeat passes, then get_fleet returns None → STOP (defensive §5 branch)
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    broker.claim_monitor_runtime(sid, os.getpid(), 5, _NOW.isoformat())
    monkeypatch.setattr("cafleet.broker.get_fleet", lambda fleet_id: None)

    pings = _stub_tmux(monkeypatch, set())
    assert monitor_tick(sid, _NOW) is STOP
    assert pings == []


def test_monitor_tick__stop_when_ownership_lost(monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    # the slot is owned by a different pid → this tick's ownership-checked
    # heartbeat matches zero rows and the loser self-terminates without pinging
    broker.claim_monitor_runtime(sid, os.getpid() + 1, 5, _NOW.isoformat())

    pings = _stub_tmux(monkeypatch, {"%0"})
    assert monitor_tick(sid, _NOW) is STOP
    assert pings == []
