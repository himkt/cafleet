"""Tests for ``monitor_tick`` — one full scan pass (§4).

The global ``_silence_real_tmux_subprocess`` fixture already stubs the tmux
``_run`` subprocess; each test additionally stubs ``list_pane_ids`` (the
per-tick liveness query) and captures the keystrokes. The loop computes the due
set over the WATCHED agents (the root Director @180 + ordinary members @720) and,
when ≥ 1 is due, wakes the dedicated monitoring member — the unenrolled watcher,
located by ``find_monitoring_member`` — exactly once via ``send_wake_trigger``
into the watcher's own pane. It never keystrokes a watched pane, so the captured
``polls`` list stays empty. ``send_poll_trigger`` still exists (``cafleet member
ping`` reuses it) but the loop never calls it.
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
    """Register an ordinary member — a watched agent enrolled @720."""
    return _register_agent(
        fleet["fleet_id"], name=name, placement=_placement(fleet, pane_id)
    )["agent_id"]


def _register_monitoring_member(fleet: dict, name: str, pane_id: str) -> int:
    """Register the dedicated monitoring member — the unenrolled watcher."""
    return broker.register_agent(
        fleet_id=fleet["fleet_id"],
        name=name,
        description="monitoring member",
        placement=_placement(fleet, pane_id),
        kind="monitoring-member",
    )["agent_id"]


def _stub_tmux(monkeypatch, live_panes, *, wake_ok=True):
    """Stub pane liveness; capture poll-trigger and wake-trigger keystrokes into
    separate lists. The loop only ever fires ``send_wake_trigger`` (into the
    watcher's pane); ``polls`` is captured to assert the loop never keystrokes a
    watched (Director / member) pane (§4). ``wake_ok`` is the boolean
    ``send_wake_trigger`` returns — pass ``False`` to model a best-effort
    keystroke that was attempted but failed to land."""
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
        return wake_ok

    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.send_poll_trigger", fake_poll
    )
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.send_wake_trigger",
        fake_wake,
        raising=False,
    )
    return polls, wakes


def test_monitor_tick__due_director_wakes_watcher(capsys, monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    watcher = _register_monitoring_member(fleet, "watcher", "%7")

    # the Director is enrolled @180 with last_ping_at=None → due immediately
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # exactly one wake, into the WATCHER's pane — never the Director's pane
    assert wakes == [("%7", sid, watcher)]
    assert polls == []
    assert ("%0", sid, director_id) not in wakes
    # record_pings advanced the due Director's cadence
    assert (
        broker.get_monitor_config(sid, director_id)["last_ping_at"] == _NOW.isoformat()
    )
    # the heartbeat was written for this tick
    assert broker.read_monitor_runtime(sid)["last_tick_at"] == _NOW.isoformat()
    # the stdout line names the due Director and routes it to a monitor wake
    out = capsys.readouterr().out
    assert f"due agent {director_id} (" in out
    assert "-> wake monitor" in out


def test_monitor_tick__due_member_wakes_watcher(capsys, monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    watcher = _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_member(fleet, "alice", "%9")

    # make the Director not-due so the member is the only due watched agent
    broker.record_ping(director_id, _NOW.isoformat())
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # one wake into the watcher's pane; no keystroke into the member's pane
    assert wakes == [("%7", sid, watcher)]
    assert ("%9", sid, member) not in wakes
    assert polls == []
    # only the due member's cadence advanced
    assert broker.get_monitor_config(sid, member)["last_ping_at"] == _NOW.isoformat()
    # the not-due Director was not in the due set
    out = capsys.readouterr().out
    assert f"due agent {member} (" in out
    assert f"due agent {director_id} (" not in out


def test_monitor_tick__multiple_due_agents_single_wake(capsys, monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    watcher = _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_member(fleet, "alice", "%9")

    # both the Director (@180) and the member (@720) are never-pinged → both due
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # exactly ONE wake into the watcher's pane, even though two agents are due
    assert wakes == [("%7", sid, watcher)]
    assert polls == []
    # both due agents' cadences advanced in the single record_pings write
    assert (
        broker.get_monitor_config(sid, director_id)["last_ping_at"] == _NOW.isoformat()
    )
    assert broker.get_monitor_config(sid, member)["last_ping_at"] == _NOW.isoformat()
    # one due-log line per due agent
    out = capsys.readouterr().out
    assert f"due agent {director_id} (" in out
    assert f"due agent {member} (" in out


def test_monitor_tick__nothing_due_no_wake_no_record(monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    _register_monitoring_member(fleet, "watcher", "%7")

    # the Director was pinged 30 s ago (< 180 s interval) → not due
    recent = (_NOW - timedelta(seconds=30)).isoformat()
    broker.record_ping(director_id, recent)
    broker.claim_monitor_runtime(sid, os.getpid(), 5, recent)
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    assert wakes == []
    assert polls == []
    # no record_pings: the Director's stamp is unchanged
    assert broker.get_monitor_config(sid, director_id)["last_ping_at"] == recent


def test_monitor_tick__no_monitoring_member_no_wake(monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    # an ordinary member is due, but there is NO monitoring member to wake
    member = _register_member(fleet, "alice", "%9")
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    assert wakes == []
    assert polls == []
    # with no watcher, nothing is recorded — the due agents keep their NULL stamp
    assert broker.get_monitor_config(sid, director_id)["last_ping_at"] is None
    assert broker.get_monitor_config(sid, member)["last_ping_at"] is None


def test_monitor_tick__watcher_pane_dead_no_wake(monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    # the Director's pane is live (so it is due), but the watcher's pane "%7"
    # is NOT in the live set → there is no live watcher to wake
    polls, wakes = _stub_tmux(monkeypatch, {"%0"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    assert wakes == []
    assert polls == []
    # nothing recorded since the watcher could not be woken
    assert broker.get_monitor_config(sid, director_id)["last_ping_at"] is None


def test_monitor_tick__failed_wake_does_not_advance_or_log(capsys, monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    watcher = _register_monitoring_member(fleet, "watcher", "%7")

    # the Director is due (never pinged) and the watcher's pane is live, so a wake
    # is ATTEMPTED — but the best-effort keystroke fails (send_wake_trigger → False)
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7"}, wake_ok=False)

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # the wake was attempted into the watcher's pane (the call returned False)
    assert wakes == [("%7", sid, watcher)]
    assert polls == []
    # a failed wake leaves the due Director flagged: last_ping_at is NOT advanced,
    # so the next tick retries instead of skipping a check for a full interval
    assert broker.get_monitor_config(sid, director_id)["last_ping_at"] is None
    # and no due-log line is emitted when the wake did not land
    out = capsys.readouterr().out
    assert "due agent" not in out
    assert "-> wake monitor" not in out


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
    # heartbeat passes, then get_fleet returns None → STOP (defensive §4 branch)
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
