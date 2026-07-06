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
from cafleet.monitor.loop import (
    CONTINUE,
    STOP,
    _flag_native_status_due,
    _last_agent_status,
    monitor_tick,
)
from tests.broker._helpers import (
    _create_fleet,
    _member_placement,
    _register_agent,
    _register_monitoring_member,
)

_NOW = datetime(2026, 6, 13, 12, 0, 0, tzinfo=UTC)


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


@pytest.fixture(autouse=True)
def _clear_native_status():
    """Reset the per-process ``last_status`` map around each test so native-state
    transition detection starts from a clean slate (mirrors ``run_monitor_loop``)."""
    _last_agent_status.clear()
    yield
    _last_agent_status.clear()


class _FakeStateMux:
    """A minimal ``AgentStateAware`` multiplexer for the native-status branch:
    reports per-pane native states and records wake keystrokes. Recognized by
    ``isinstance(mux, AgentStateAware)`` because it defines both capability
    methods."""

    name = "herdr"

    def __init__(self, live_panes, statuses, *, wake_ok=True):
        self._live = set(live_panes)
        self._statuses = statuses
        self._wake_ok = wake_ok
        self.wakes: list[tuple] = []

    def list_pane_ids(self):
        return set(self._live)

    def agent_status(self, *, target_pane_id):
        return self._statuses.get(target_pane_id)

    def wait_agent_status(self, *, target_pane_id, status, timeout_ms):
        return False

    def send_wake_trigger(self, *, target_pane_id, due_agents, director_agent_id):
        self.wakes.append(
            (target_pane_id, [t["agent_id"] for t in due_agents], director_agent_id)
        )
        return self._wake_ok


def _native_target(
    agent_id, pane_id="%9", *, pane_alive=True, name="alice", enabled=True
):
    return {
        "agent_id": agent_id,
        "pane_id": pane_id,
        "pane_alive": pane_alive,
        "name": name,
        "enabled": enabled,
    }


def _register_member(fleet: dict, name: str, pane_id: str) -> int:
    """Register an ordinary member — a watched agent enrolled @720."""
    return _register_agent(
        fleet["fleet_id"],
        name=name,
        placement=_member_placement(fleet["director"]["agent_id"], pane_id),
    )["agent_id"]


def _stub_tmux(monkeypatch, live_panes, *, wake_ok=True):
    """Stub pane liveness; capture poll-trigger and wake-trigger keystrokes into
    separate lists. The loop only ever fires ``send_wake_trigger`` (into the
    watcher's pane); ``polls`` is captured to assert the loop never keystrokes a
    watched (Director / member) pane (§4). ``wake_ok`` is the boolean
    ``send_wake_trigger`` returns — pass ``False`` to model a best-effort
    keystroke that was attempted but failed to land. Each ``wakes`` entry is
    ``(target_pane_id, [conveyed due-agent ids], director_agent_id)`` — the due
    set and the Director id the wake nudge names (§2/§4)."""
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

    def fake_wake(self, *, target_pane_id, due_agents, director_agent_id):
        wakes.append(
            (target_pane_id, [t["agent_id"] for t in due_agents], director_agent_id)
        )
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
    _register_monitoring_member(fleet, "watcher", "%7")

    # the Director is enrolled @180 with last_ping_at=None → due immediately
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # exactly one wake into the WATCHER's pane ("%7"), conveying the due Director
    # as the sole due agent plus the Director id; the loop never keystrokes a
    # watched pane (asserted by ``polls == []``).
    assert wakes == [("%7", [director_id], director_id)]
    assert polls == []
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
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_member(fleet, "alice", "%9")

    # make the Director not-due so the member is the only due watched agent
    broker.record_pings([director_id], _NOW.isoformat())
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # one wake into the watcher's pane conveying the due MEMBER (not the
    # Director) plus the correct director_agent_id; no keystroke into a watched
    # pane (``polls == []``).
    assert wakes == [("%7", [member], director_id)]
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
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_member(fleet, "alice", "%9")

    # both the Director (@180) and the member (@720) are never-pinged → both due
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # exactly ONE wake into the watcher's pane, even though two agents are due —
    # the single wake conveys BOTH due ids (order-independent) plus the Director.
    assert len(wakes) == 1
    pane_id, conveyed_ids, conveyed_director = wakes[0]
    assert pane_id == "%7"
    assert set(conveyed_ids) == {director_id, member}
    assert conveyed_director == director_id
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
    broker.record_pings([director_id], recent)
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
    _register_monitoring_member(fleet, "watcher", "%7")

    # the Director is due (never pinged) and the watcher's pane is live, so a wake
    # is ATTEMPTED — but the best-effort keystroke fails (send_wake_trigger → False)
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7"}, wake_ok=False)

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # the wake was attempted into the watcher's pane (the call returned False),
    # conveying the due Director and the Director id
    assert wakes == [("%7", [director_id], director_id)]
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


# --- native agent-state due trigger (§5, herdr-only) -----------------------


def test_flag_native_status_due__blocked_transition_flags_with_reason():
    """A transition into ``blocked`` flags the agent due and tags it with the
    ``status:blocked`` wake reason (unioned onto the interval-due set). The read
    statuses are RETURNED (not committed here) — the caller commits them only
    after a successful wake."""
    mux = _FakeStateMux({"%9"}, {"%9": "blocked"})
    targets = [_native_target(5)]
    due: list[dict] = []
    read = _flag_native_status_due(mux, targets, due)
    assert [t["agent_id"] for t in due] == [5]
    assert due[0]["wake_reason"] == "status:blocked"
    # returns the read status; does NOT mutate the module-level last-seen map.
    assert read == {5: "blocked"}
    assert 5 not in _last_agent_status


def test_flag_native_status_due__done_transition_flags_with_reason():
    mux = _FakeStateMux({"%9"}, {"%9": "done"})
    targets = [_native_target(5)]
    due: list[dict] = []
    _flag_native_status_due(mux, targets, due)
    assert due[0]["wake_reason"] == "status:done"


@pytest.mark.parametrize("status", ["working", "idle", "unknown", None])
def test_flag_native_status_due__non_attention_status_not_flagged(status):
    """Only ``blocked``/``done`` are attention states; every other native state
    (and no-agent ``None``) leaves the agent unflagged."""
    mux = _FakeStateMux({"%9"}, {"%9": status})
    targets = [_native_target(5)]
    due: list[dict] = []
    _flag_native_status_due(mux, targets, due)
    assert due == []


def test_flag_native_status_due__same_attention_status_wakes_only_once():
    """One ``blocked`` episode wakes once: once the caller commits the read status
    to ``_last_agent_status`` (as ``monitor_tick`` does after a successful wake), a
    second tick with the same status is a non-transition (prev == current) and
    does not re-flag."""
    mux = _FakeStateMux({"%9"}, {"%9": "blocked"})
    targets = [_native_target(5)]
    first: list[dict] = []
    read = _flag_native_status_due(mux, targets, first)
    assert [t["agent_id"] for t in first] == [5]
    # the caller commits the read statuses only after a successful wake
    _last_agent_status.update(read)
    second: list[dict] = []
    _flag_native_status_due(mux, targets, second)
    assert second == []


def test_flag_native_status_due__uncommitted_status_re_flags_next_call():
    """If the caller does NOT commit (a failed/absent wake), the same ``blocked``
    status re-flags on the next call — the episode is not consumed."""
    mux = _FakeStateMux({"%9"}, {"%9": "blocked"})
    targets = [_native_target(5)]
    first: list[dict] = []
    _flag_native_status_due(mux, targets, first)
    assert [t["agent_id"] for t in first] == [5]
    # no commit → prev stays None → still a transition
    second: list[dict] = []
    _flag_native_status_due(mux, targets, second)
    assert [t["agent_id"] for t in second] == [5]


def test_flag_native_status_due__disabled_target_not_read_or_flagged():
    """A monitor-disabled target is skipped entirely (mirrors ``should_ping``):
    it is never point-read and never flagged, even when its native status is an
    attention state."""
    mux = _FakeStateMux({"%9"}, {"%9": "blocked"})
    targets = [_native_target(5, enabled=False)]
    due: list[dict] = []
    read = _flag_native_status_due(mux, targets, due)
    assert due == []
    assert read == {}


def test_flag_native_status_due__recovery_read_committed_immediately_rearms_episode():
    """blocked → working → blocked across three no-wake ticks. The flagged
    ``blocked`` read is RETURNED (pending a wake, uncommitted), but the
    NON-flagged ``working`` recovery read is committed to ``_last_agent_status``
    IMMEDIATELY — even on a no-wake tick — so the second ``blocked`` is a real
    transition (prev == ``working``) and natively flags again.

    Contrast ``..._uncommitted_status_re_flags_next_call`` (a *flagged* episode
    that a wake failure leaves un-consumed): there ``blocked`` stays ``blocked``
    with no commit, and prev stays ``None``. Here the *recovery* commits on its
    own, which is what re-arms detection of the next distinct episode."""
    targets = [_native_target(5)]

    # Tick 1: blocked → flagged, returned as pending, NOT committed (awaits a wake).
    due1: list[dict] = []
    pending1 = _flag_native_status_due(
        _FakeStateMux({"%9"}, {"%9": "blocked"}), targets, due1
    )
    assert [t["agent_id"] for t in due1] == [5]
    assert pending1 == {5: "blocked"}
    assert 5 not in _last_agent_status  # a flagged read is not self-committed

    # No live watcher → woke=False → the caller does NOT commit pending1.

    # Tick 2: working recovery → NON-attention → committed IMMEDIATELY, not flagged.
    due2: list[dict] = []
    pending2 = _flag_native_status_due(
        _FakeStateMux({"%9"}, {"%9": "working"}), targets, due2
    )
    assert due2 == []
    assert pending2 == {}
    assert _last_agent_status[5] == "working"  # recovery recorded on a no-wake tick

    # Tick 3: blocked again → prev == "working" → transition → flags again.
    due3: list[dict] = []
    pending3 = _flag_native_status_due(
        _FakeStateMux({"%9"}, {"%9": "blocked"}), targets, due3
    )
    assert [t["agent_id"] for t in due3] == [5]
    assert pending3 == {5: "blocked"}


def test_flag_native_status_due__already_interval_due_not_duplicated():
    """An agent already in the interval-due set is not appended twice, and keeps
    no native wake reason (the interval trigger owns it)."""
    mux = _FakeStateMux({"%9"}, {"%9": "blocked"})
    target = _native_target(5)
    due = [target]
    _flag_native_status_due(mux, [target], due)
    assert [t["agent_id"] for t in due] == [5]
    assert "wake_reason" not in target


def test_flag_native_status_due__dead_or_pending_pane_skipped():
    """Agents with no pane (pending) or a dead pane are never point-read."""
    mux = _FakeStateMux(set(), {})
    targets = [
        _native_target(5, pane_id=None),
        _native_target(6, pane_alive=False),
    ]
    due: list[dict] = []
    _flag_native_status_due(mux, targets, due)
    assert due == []


def test_monitor_tick__native_blocked_transition_wakes_watcher(capsys, monkeypatch):
    """End-to-end on an AgentStateAware backend: a member that is NOT interval-due
    but whose native status just entered ``blocked`` is unioned into the due set,
    wakes the watcher, and logs the ``[status:blocked]`` wake-reason suffix."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_member(fleet, "alice", "%9")

    # Make BOTH watched agents interval-not-due so only the native transition can
    # flag the member.
    broker.record_pings([director_id, member], _NOW.isoformat())
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    fake = _FakeStateMux({"%0", "%7", "%9"}, {"%0": "idle", "%9": "blocked"})
    monkeypatch.setattr("cafleet.monitor.loop.resolve_multiplexer", lambda: fake)

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # the member's native blocked transition flags it due → one wake naming alice
    assert fake.wakes == [("%7", [member], director_id)]
    # native-due agents carry the status wake-reason suffix on the stdout line
    out = capsys.readouterr().out
    assert f"due agent {member} (" in out
    assert "[status:blocked]" in out
    assert "-> wake monitor" in out
    # the successful wake advanced the member's cadence
    assert broker.get_monitor_config(sid, member)["last_ping_at"] == _NOW.isoformat()


def test_monitor_tick__native_branch_inert_on_tmux_backend(monkeypatch):
    """On the tmux backend (not AgentStateAware), the native-status branch never
    runs: with nobody interval-due there is no wake, so a would-be ``blocked``
    member cannot be flagged natively — the interval-only behavior is preserved."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_member(fleet, "alice", "%9")

    broker.record_pings([director_id, member], _NOW.isoformat())
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    # resolve_multiplexer returns the tmux singleton (autouse-pinned); _stub_tmux
    # patches its class methods. tmux is not AgentStateAware → branch skipped.
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    assert wakes == []
    assert polls == []


def test_monitor_tick__native_transition_not_consumed_when_no_wake(monkeypatch):
    """When there is no live watcher to wake, the native ``blocked`` transition is
    NOT consumed: ``_last_agent_status`` is left uncommitted, so the SAME
    transition re-flags and wakes on the next tick once a watcher is live."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_member(fleet, "alice", "%9")
    broker.record_pings([director_id, member], _NOW.isoformat())
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )

    # Tick 1: the watcher's pane "%7" is NOT live, so the wake block is skipped and
    # nothing is committed — the blocked episode stays un-consumed.
    fake1 = _FakeStateMux({"%0", "%9"}, {"%0": "idle", "%9": "blocked"})
    monkeypatch.setattr("cafleet.monitor.loop.resolve_multiplexer", lambda: fake1)
    assert monitor_tick(sid, _NOW) is CONTINUE
    assert fake1.wakes == []
    assert member not in _last_agent_status

    # Tick 2: same blocked status, watcher now live → the un-consumed transition
    # re-flags and wakes.
    fake2 = _FakeStateMux({"%0", "%7", "%9"}, {"%0": "idle", "%9": "blocked"})
    monkeypatch.setattr("cafleet.monitor.loop.resolve_multiplexer", lambda: fake2)
    assert monitor_tick(sid, _NOW) is CONTINUE
    assert fake2.wakes == [("%7", [member], director_id)]


def test_monitor_tick__native_transition_consumed_on_successful_wake(monkeypatch):
    """A successful wake commits the read statuses, so the SAME ``blocked`` status
    does not re-wake on the next tick — the episode wakes exactly once."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_member(fleet, "alice", "%9")
    broker.record_pings([director_id, member], _NOW.isoformat())
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )

    # Tick 1: blocked transition wakes the live watcher and commits the status.
    fake1 = _FakeStateMux({"%0", "%7", "%9"}, {"%0": "idle", "%9": "blocked"})
    monkeypatch.setattr("cafleet.monitor.loop.resolve_multiplexer", lambda: fake1)
    assert monitor_tick(sid, _NOW) is CONTINUE
    assert fake1.wakes == [("%7", [member], director_id)]
    assert _last_agent_status[member] == "blocked"

    # Tick 2 (1 s later, still interval-not-due): same blocked status is a
    # non-transition → no native flag → no wake.
    later = _NOW + timedelta(seconds=1)
    fake2 = _FakeStateMux({"%0", "%7", "%9"}, {"%0": "idle", "%9": "blocked"})
    monkeypatch.setattr("cafleet.monitor.loop.resolve_multiplexer", lambda: fake2)
    assert monitor_tick(sid, later) is CONTINUE
    assert fake2.wakes == []
