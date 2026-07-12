"""Tests for ``monitor_tick`` — one full scan pass (§4).

The global ``_silence_real_tmux_subprocess`` fixture already stubs the tmux
``_run`` subprocess; each test additionally stubs ``list_pane_ids`` (the
per-tick liveness query) and captures the keystrokes. The loop computes the due
set over the WATCHED members (the root Director @180 + ordinary members @720) and,
when ≥ 1 is due, wakes the dedicated monitoring member — the unenrolled watcher,
located by ``find_monitoring_member`` — exactly once via ``send_wake_trigger``
into the watcher's own pane. It never keystrokes a watched pane, so the captured
``polls`` list stays empty. ``send_poll_trigger`` still exists (``cafleet member
ping`` reuses it) but the loop never calls it.
"""

import os
from datetime import UTC, datetime, timedelta

import click
import pytest

from cafleet import broker
from cafleet.config import settings
from cafleet.db.models import Fleet
from cafleet.monitor.loop import (
    _WAKE_ON_STATUS,
    CONTINUE,
    STOP,
    _flag_native_status_due,
    _flag_stall_check_due,
    _last_member_status,
    _last_stall_check_at,
    monitor_tick,
    run_monitor_loop,
)
from tests.broker._helpers import (
    _create_fleet,
    _member_placement,
    _register_member,
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
    _last_member_status.clear()
    yield
    _last_member_status.clear()


@pytest.fixture(autouse=True)
def _clear_stall_check():
    """Reset the per-process stall-check baseline map around each test (mirrors
    ``run_monitor_loop``'s per-run clear), so stall-check due-ness starts fresh."""
    _last_stall_check_at.clear()
    yield
    _last_stall_check_at.clear()


@pytest.fixture(autouse=True)
def _default_stall_disabled(monkeypatch):
    """Disable stall detection by default (``monitor_stall_interval == 0``) so the
    interval and native-status tests keep their exact wake semantics — the
    stall-check trigger fires on neither backend when the interval is 0. The
    stall-specific tests opt in by setting a non-zero interval themselves; their
    ``setattr`` runs after this autouse fixture and wins for that test."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 0)


class _FakeStateMux:
    """A minimal ``AgentStateAware`` multiplexer for the native-status branch:
    reports per-pane native states and records wake keystrokes. Recognized by
    ``isinstance(mux, AgentStateAware)`` because it defines the capability
    method."""

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

    def send_wake_trigger(self, *, target_pane_id, due_members, director_member_id):
        self.wakes.append(
            (target_pane_id, [t["member_id"] for t in due_members], director_member_id)
        )
        return self._wake_ok


def _native_target(
    member_id, pane_id="%9", *, pane_alive=True, name="alice", enabled=True
):
    return {
        "member_id": member_id,
        "pane_id": pane_id,
        "pane_alive": pane_alive,
        "name": name,
        "enabled": enabled,
    }


def _register_watched_member(fleet: dict, name: str, pane_id: str) -> int:
    """Register an ordinary member — a watched member enrolled @720."""
    return _register_member(
        fleet["fleet_id"],
        name=name,
        placement=_member_placement(pane_id),
    )["member_id"]


def _stub_tmux(monkeypatch, live_panes, *, wake_ok=True):
    """Stub pane liveness; capture poll-trigger and wake-trigger keystrokes into
    separate lists. The loop only ever fires ``send_wake_trigger`` (into the
    watcher's pane); ``polls`` is captured to assert the loop never keystrokes a
    watched (Director / member) pane (§4). ``wake_ok`` is the boolean
    ``send_wake_trigger`` returns — pass ``False`` to model a best-effort
    keystroke that was attempted but failed to land. Each ``wakes`` entry is
    ``(target_pane_id, [conveyed due-member ids], director_member_id)`` — the due
    set and the Director id the wake nudge names (§2/§4)."""
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.list_pane_ids",
        lambda self: set(live_panes),
        raising=False,
    )
    polls = []
    wakes = []

    def fake_poll(self, *, target_pane_id, fleet_id, member_id):
        polls.append((target_pane_id, fleet_id, member_id))
        return True

    def fake_wake(self, *, target_pane_id, due_members, director_member_id):
        wakes.append(
            (target_pane_id, [t["member_id"] for t in due_members], director_member_id)
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


def _stub_tmux_wakes(monkeypatch, live_panes, *, wake_ok=True):
    """Stub pane liveness and capture the FULL per-member wake payload.

    Unlike ``_stub_tmux`` (which records only conveyed ids), each ``wakes`` entry
    is ``(target_pane_id, [(member_id, [wake_reasons…]), …], director_member_id)`` —
    so a test can assert the per-member ``wake_reasons`` plumbing the loop attaches
    (``interval`` / ``status:done`` / ``stall-check``). The reasons list is
    snapshotted at call time because the loop reuses the mutable target dicts."""
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.list_pane_ids",
        lambda self: set(live_panes),
        raising=False,
    )
    wakes = []

    def fake_wake(self, *, target_pane_id, due_members, director_member_id):
        wakes.append(
            (
                target_pane_id,
                [
                    (t["member_id"], list(t.get("wake_reasons", [])))
                    for t in due_members
                ],
                director_member_id,
            )
        )
        return wake_ok

    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.send_wake_trigger",
        fake_wake,
        raising=False,
    )
    return wakes


def test_monitor_tick__due_director_wakes_watcher(capsys, monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")

    # the Director is enrolled @180 with last_ping_at=None → due immediately
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # exactly one wake into the WATCHER's pane ("%7"), conveying the due Director
    # as the sole due member plus the Director id; the loop never keystrokes a
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
    assert f"due member {director_id} (" in out
    assert "-> wake monitor" in out


def test_monitor_tick__due_member_wakes_watcher(capsys, monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")

    # make the Director not-due so the member is the only due watched member
    broker.record_pings([director_id], _NOW.isoformat())
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # one wake into the watcher's pane conveying the due MEMBER (not the
    # Director) plus the correct director_member_id; no keystroke into a watched
    # pane (``polls == []``).
    assert wakes == [("%7", [member], director_id)]
    assert polls == []
    # only the due member's cadence advanced
    assert broker.get_monitor_config(sid, member)["last_ping_at"] == _NOW.isoformat()
    # the not-due Director was not in the due set
    out = capsys.readouterr().out
    assert f"due member {member} (" in out
    assert f"due member {director_id} (" not in out


def test_monitor_tick__multiple_due_members_single_wake(capsys, monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")

    # both the Director (@180) and the member (@720) are never-pinged → both due
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%7", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # exactly ONE wake into the watcher's pane, even though two members are due —
    # the single wake conveys BOTH due ids (order-independent) plus the Director.
    assert len(wakes) == 1
    pane_id, conveyed_ids, conveyed_director = wakes[0]
    assert pane_id == "%7"
    assert set(conveyed_ids) == {director_id, member}
    assert conveyed_director == director_id
    assert polls == []
    # both due members' cadences advanced in the single record_pings write
    assert (
        broker.get_monitor_config(sid, director_id)["last_ping_at"] == _NOW.isoformat()
    )
    assert broker.get_monitor_config(sid, member)["last_ping_at"] == _NOW.isoformat()
    # one due-log line per due member
    out = capsys.readouterr().out
    assert f"due member {director_id} (" in out
    assert f"due member {member} (" in out


def test_monitor_tick__nothing_due_no_wake_no_record(monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
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
    director_id = fleet["director"]["member_id"]
    # an ordinary member is due, but there is NO monitoring member to wake
    member = _register_watched_member(fleet, "alice", "%9")
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    polls, wakes = _stub_tmux(monkeypatch, {"%0", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    assert wakes == []
    assert polls == []
    # with no watcher, nothing is recorded — the due members keep their NULL stamp
    assert broker.get_monitor_config(sid, director_id)["last_ping_at"] is None
    assert broker.get_monitor_config(sid, member)["last_ping_at"] is None


def test_monitor_tick__watcher_pane_dead_no_wake(monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
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
    director_id = fleet["director"]["member_id"]
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
    assert "due member" not in out
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
#
# ``_WAKE_ON_STATUS = ("done",)``. A transition into
# ``done`` still flags a wake (tagged ``status:done``); a transition into
# ``blocked`` is RECORDED in ``_last_member_status`` but NEVER flags a wake — a
# member parked on a user answer must not be woken about. The per-member wake tag
# is a ``wake_reasons`` LIST.


def test_wake_on_status_is_done_only():
    """The wake-on-status set is exactly ``("done",)`` — ``blocked`` is not
    a wake trigger."""
    assert _WAKE_ON_STATUS == ("done",)
    assert "blocked" not in _WAKE_ON_STATUS


def test_flag_native_status_due__done_transition_flags_with_reason():
    """A transition into ``done`` flags the member due and tags it with the
    ``status:done`` wake reason as a one-element ``wake_reasons`` list. The read
    statuses are RETURNED (not committed here) — the caller commits them only
    after a successful wake."""
    mux = _FakeStateMux({"%9"}, {"%9": "done"})
    targets = [_native_target(5)]
    due: list[dict] = []
    read = _flag_native_status_due(mux, targets, due)
    assert [t["member_id"] for t in due] == [5]
    assert due[0]["wake_reasons"] == ["status:done"]
    # returns the read status; does NOT mutate the module-level last-seen map.
    assert read == {5: "done"}
    assert 5 not in _last_member_status


def test_flag_native_status_due__blocked_transition_recorded_but_not_flagged():
    """A transition into ``blocked`` NEVER flags a wake (the destructive
    awaiting-user path this design closes), yet the read IS committed to
    ``_last_member_status`` immediately — so the episode is tracked and a later
    ``blocked → working`` recovery is still seen as a transition. Nothing is
    returned as pending, because there is no wake to gate."""
    mux = _FakeStateMux({"%9"}, {"%9": "blocked"})
    targets = [_native_target(5)]
    due: list[dict] = []
    read = _flag_native_status_due(mux, targets, due)
    assert due == []
    assert read == {}
    assert _last_member_status[5] == "blocked"


@pytest.mark.parametrize("status", ["working", "idle", "unknown", "blocked", None])
def test_flag_native_status_due__non_wake_status_not_flagged(status):
    """``done`` is the sole wake-on-status state; every other native state —
    including ``blocked`` (recorded, never woken) and no-agent ``None`` — leaves
    the member unflagged."""
    mux = _FakeStateMux({"%9"}, {"%9": status})
    targets = [_native_target(5)]
    due: list[dict] = []
    _flag_native_status_due(mux, targets, due)
    assert due == []


def test_flag_native_status_due__same_done_status_wakes_only_once():
    """One ``done`` episode wakes once: once the caller commits the read status to
    ``_last_member_status`` (as ``monitor_tick`` does after a successful wake), a
    second tick with the same status is a non-transition (prev == current) and
    does not re-flag."""
    mux = _FakeStateMux({"%9"}, {"%9": "done"})
    targets = [_native_target(5)]
    first: list[dict] = []
    read = _flag_native_status_due(mux, targets, first)
    assert [t["member_id"] for t in first] == [5]
    # the caller commits the read statuses only after a successful wake
    _last_member_status.update(read)
    second: list[dict] = []
    _flag_native_status_due(mux, targets, second)
    assert second == []


def test_flag_native_status_due__uncommitted_done_re_flags_next_call():
    """If the caller does NOT commit (a failed/absent wake), the same ``done``
    status re-flags on the next call — the episode is not consumed."""
    mux = _FakeStateMux({"%9"}, {"%9": "done"})
    targets = [_native_target(5)]
    first: list[dict] = []
    _flag_native_status_due(mux, targets, first)
    assert [t["member_id"] for t in first] == [5]
    # no commit → prev stays None → still a transition
    second: list[dict] = []
    _flag_native_status_due(mux, targets, second)
    assert [t["member_id"] for t in second] == [5]


def test_flag_native_status_due__disabled_target_not_read_or_flagged():
    """A monitor-disabled target is skipped entirely (mirrors ``should_ping``):
    it is never point-read and never flagged, even when its native status is the
    ``done`` wake state."""
    mux = _FakeStateMux({"%9"}, {"%9": "done"})
    targets = [_native_target(5, enabled=False)]
    due: list[dict] = []
    read = _flag_native_status_due(mux, targets, due)
    assert due == []
    assert read == {}


def test_flag_native_status_due__recovery_read_committed_immediately_rearms_episode():
    """done → working → done across three no-wake ticks. The flagged ``done`` read
    is RETURNED (pending a wake, uncommitted), but the NON-flagged ``working``
    recovery read is committed to ``_last_member_status`` IMMEDIATELY — even on a
    no-wake tick — so the second ``done`` is a real transition (prev ==
    ``working``) and natively flags again.

    Contrast ``..._uncommitted_done_re_flags_next_call`` (a *flagged* episode that
    a wake failure leaves un-consumed): there ``done`` stays ``done`` with no
    commit, and prev stays ``None``. Here the *recovery* commits on its own, which
    is what re-arms detection of the next distinct episode."""
    targets = [_native_target(5)]

    # Tick 1: done → flagged, returned as pending, NOT committed (awaits a wake).
    due1: list[dict] = []
    pending1 = _flag_native_status_due(
        _FakeStateMux({"%9"}, {"%9": "done"}), targets, due1
    )
    assert [t["member_id"] for t in due1] == [5]
    assert pending1 == {5: "done"}
    assert 5 not in _last_member_status  # a flagged read is not self-committed

    # No live watcher → woke=False → the caller does NOT commit pending1.

    # Tick 2: working recovery → NON-wake → committed IMMEDIATELY, not flagged.
    due2: list[dict] = []
    pending2 = _flag_native_status_due(
        _FakeStateMux({"%9"}, {"%9": "working"}), targets, due2
    )
    assert due2 == []
    assert pending2 == {}
    assert _last_member_status[5] == "working"  # recovery recorded on a no-wake tick

    # Tick 3: done again → prev == "working" → transition → flags again.
    due3: list[dict] = []
    pending3 = _flag_native_status_due(
        _FakeStateMux({"%9"}, {"%9": "done"}), targets, due3
    )
    assert [t["member_id"] for t in due3] == [5]
    assert pending3 == {5: "done"}


def test_flag_native_status_due__already_interval_due_keeps_only_interval_reason():
    """A member already in the interval-due set is not appended twice, and the
    native ``done`` trigger does not re-tag it — it keeps only its ``interval``
    reason (the interval trigger owns it)."""
    mux = _FakeStateMux({"%9"}, {"%9": "done"})
    target = _native_target(5)
    target["wake_reasons"] = ["interval"]
    due = [target]
    _flag_native_status_due(mux, [target], due)
    assert [t["member_id"] for t in due] == [5]
    assert target["wake_reasons"] == ["interval"]


def test_flag_native_status_due__dead_or_pending_pane_skipped():
    """Members with no pane (pending) or a dead pane are never point-read."""
    mux = _FakeStateMux(set(), {})
    targets = [
        _native_target(5, pane_id=None),
        _native_target(6, pane_alive=False),
    ]
    due: list[dict] = []
    _flag_native_status_due(mux, targets, due)
    assert due == []


def test_monitor_tick__native_done_transition_wakes_watcher(capsys, monkeypatch):
    """End-to-end on an AgentStateAware backend: a member that is NOT interval-due
    but whose native status just entered ``done`` is unioned into the due set,
    wakes the watcher, and logs the ``[status:done]`` wake-reason suffix. (Stall
    detection is disabled by the autouse fixture, so only the native ``done``
    transition can flag the member.)"""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")

    # Make BOTH watched members interval-not-due so only the native transition can
    # flag the member.
    broker.record_pings([director_id, member], _NOW.isoformat())
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    fake = _FakeStateMux({"%0", "%7", "%9"}, {"%0": "idle", "%9": "done"})
    monkeypatch.setattr("cafleet.monitor.loop.resolve_multiplexer", lambda: fake)

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # the member's native done transition flags it due → one wake naming alice
    assert fake.wakes == [("%7", [member], director_id)]
    # native-due members carry the status wake-reason suffix on the stdout line
    out = capsys.readouterr().out
    assert f"due member {member} (" in out
    assert "[status:done]" in out
    assert "-> wake monitor" in out
    # the successful wake advanced the member's cadence (status:done → record_pings)
    assert broker.get_monitor_config(sid, member)["last_ping_at"] == _NOW.isoformat()


def test_monitor_tick__native_blocked_transition_does_not_wake(monkeypatch):
    """A member whose native status just entered ``blocked`` is NOT woken about —
    ``blocked`` is not in ``_WAKE_ON_STATUS``. No wake fires, yet the ``blocked``
    read is recorded in ``_last_member_status`` so the episode is tracked. This is
    the Finding-B closure: a member awaiting a user answer is never re-engaged."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")

    # Both interval-not-due, stall detection disabled → only a native trigger could
    # flag the member, and ``blocked`` is not one.
    recent = _NOW.isoformat()
    broker.record_pings([director_id, member], recent)
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    fake = _FakeStateMux({"%0", "%7", "%9"}, {"%0": "idle", "%9": "blocked"})
    monkeypatch.setattr("cafleet.monitor.loop.resolve_multiplexer", lambda: fake)

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    assert fake.wakes == []
    # the blocked read is committed immediately (episode tracked for later recovery)
    assert _last_member_status[member] == "blocked"
    # the member's cadence is untouched — it was never flagged
    assert broker.get_monitor_config(sid, member)["last_ping_at"] == recent


def test_monitor_tick__native_branch_inert_on_tmux_backend(monkeypatch):
    """On the tmux backend (not AgentStateAware), the native-status branch never
    runs: with nobody interval-due and stall detection disabled there is no wake,
    so a would-be ``done`` member cannot be flagged natively — the interval-only
    behavior is preserved."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")

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
    """When there is no live watcher to wake, the native ``done`` transition is
    NOT consumed: ``_last_member_status`` is left uncommitted, so the SAME
    transition re-flags and wakes on the next tick once a watcher is live."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")
    broker.record_pings([director_id, member], _NOW.isoformat())
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )

    # Tick 1: the watcher's pane "%7" is NOT live, so the wake block is skipped and
    # nothing is committed — the done episode stays un-consumed.
    fake1 = _FakeStateMux({"%0", "%9"}, {"%0": "idle", "%9": "done"})
    monkeypatch.setattr("cafleet.monitor.loop.resolve_multiplexer", lambda: fake1)
    assert monitor_tick(sid, _NOW) is CONTINUE
    assert fake1.wakes == []
    assert member not in _last_member_status

    # Tick 2: same done status, watcher now live → the un-consumed transition
    # re-flags and wakes.
    fake2 = _FakeStateMux({"%0", "%7", "%9"}, {"%0": "idle", "%9": "done"})
    monkeypatch.setattr("cafleet.monitor.loop.resolve_multiplexer", lambda: fake2)
    assert monitor_tick(sid, _NOW) is CONTINUE
    assert fake2.wakes == [("%7", [member], director_id)]


def test_monitor_tick__native_transition_consumed_on_successful_wake(monkeypatch):
    """A successful wake commits the read statuses, so the SAME ``done`` status
    does not re-wake on the next tick — the episode wakes exactly once."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")
    broker.record_pings([director_id, member], _NOW.isoformat())
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )

    # Tick 1: done transition wakes the live watcher and commits the status.
    fake1 = _FakeStateMux({"%0", "%7", "%9"}, {"%0": "idle", "%9": "done"})
    monkeypatch.setattr("cafleet.monitor.loop.resolve_multiplexer", lambda: fake1)
    assert monitor_tick(sid, _NOW) is CONTINUE
    assert fake1.wakes == [("%7", [member], director_id)]
    assert _last_member_status[member] == "done"

    # Tick 2 (1 s later, still interval-not-due): same done status is a
    # non-transition → no native flag → no wake.
    later = _NOW + timedelta(seconds=1)
    fake2 = _FakeStateMux({"%0", "%7", "%9"}, {"%0": "idle", "%9": "done"})
    monkeypatch.setattr("cafleet.monitor.loop.resolve_multiplexer", lambda: fake2)
    assert monitor_tick(sid, later) is CONTINUE
    assert fake2.wakes == []


# --- stall-detection cadence (§ Stall-detection cadence) -------------------
#
# A separate per-member cadence driven by ``settings.monitor_stall_interval``
# (default 240; 0 disables). ``_flag_stall_check_due(targets, due, now)`` unions
# each enabled, live, stall-check-due watched member into the due set with a
# ``stall-check`` reason; a member is stall-check due when it is ABSENT from
# ``_last_stall_check_at`` (first-tick) or ``now - _last_stall_check_at[id] >=
# interval``. The baseline is committed only on a successful wake (in
# ``monitor_tick``), and a stall-check-only member is excluded from ``record_pings``
# so the two cadences stay independent.


def test_flag_stall_check_due__first_tick_flags_when_absent(monkeypatch):
    """A member absent from ``_last_stall_check_at`` is stall-check due on the
    first observation (mirrors ``should_ping``'s ``last_ping_at is None → due``);
    it is tagged ``stall-check`` and the dict is NOT self-committed here."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    targets = [_native_target(5)]
    due: list[dict] = []
    _flag_stall_check_due(targets, due, _NOW)
    assert [t["member_id"] for t in due] == [5]
    assert due[0]["wake_reasons"] == ["stall-check"]
    # the commit is gated on a successful wake in monitor_tick, not done here.
    assert 5 not in _last_stall_check_at


def test_flag_stall_check_due__not_due_before_interval(monkeypatch):
    """Within one interval of the last stall-check the member is not re-flagged."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    _last_stall_check_at[5] = _NOW - timedelta(seconds=100)
    targets = [_native_target(5)]
    due: list[dict] = []
    _flag_stall_check_due(targets, due, _NOW)
    assert due == []


def test_flag_stall_check_due__due_once_interval_elapsed(monkeypatch):
    """At exactly one interval since the last stall-check the member is due again."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    _last_stall_check_at[5] = _NOW - timedelta(seconds=240)
    targets = [_native_target(5)]
    due: list[dict] = []
    _flag_stall_check_due(targets, due, _NOW)
    assert [t["member_id"] for t in due] == [5]
    assert due[0]["wake_reasons"] == ["stall-check"]


def test_flag_stall_check_due__interval_zero_disables(monkeypatch):
    """``monitor_stall_interval == 0`` disables the trigger entirely — no member is
    flagged and no ``stall-check`` reason is ever emitted."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 0)
    targets = [_native_target(5)]
    due: list[dict] = []
    _flag_stall_check_due(targets, due, _NOW)
    assert due == []


def test_flag_stall_check_due__appends_reason_to_already_due_member(monkeypatch):
    """A stall-check-due member that is ALSO interval-due is not appended twice; the
    ``stall-check`` reason is UNIONED onto its existing reasons (so it is both
    ping-recorded and stall-baseline-committed)."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    target = _native_target(5)
    target["wake_reasons"] = ["interval"]
    due = [target]
    _flag_stall_check_due([target], due, _NOW)
    assert [t["member_id"] for t in due] == [5]
    assert target["wake_reasons"] == ["interval", "stall-check"]


def test_flag_stall_check_due__disabled_target_skipped(monkeypatch):
    """A monitor-disabled target is never stall-check flagged (mirrors ``should_ping``)."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    targets = [_native_target(5, enabled=False)]
    due: list[dict] = []
    _flag_stall_check_due(targets, due, _NOW)
    assert due == []


def test_flag_stall_check_due__dead_or_pending_pane_skipped(monkeypatch):
    """Members with no pane (pending) or a dead pane are never stall-check flagged."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    targets = [
        _native_target(5, pane_id=None),
        _native_target(6, pane_alive=False),
    ]
    due: list[dict] = []
    _flag_stall_check_due(targets, due, _NOW)
    assert due == []


def test_monitor_tick__stall_check_only_member_excluded_from_record_pings(monkeypatch):
    """A stall-check-only member (interval-not-due, flagged only by the stall
    cadence) is EXCLUDED from ``record_pings`` — its ``last_ping_at`` interval
    cadence is untouched — while its stall baseline IS committed. An interval-due
    member in the same wake is ping-recorded as usual. This keeps the two cadences
    independent."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")

    # Director interval-due (never pinged); member interval-not-due (pinged now).
    recent = _NOW.isoformat()
    broker.record_pings([member], recent)
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    wakes = _stub_tmux_wakes(monkeypatch, {"%0", "%7", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    # one wake; the member is conveyed with a stall-check-only reason set, the
    # Director with interval (both are also first-tick stall-check due).
    assert len(wakes) == 1
    conveyed = dict(wakes[0][1])
    assert "stall-check" in conveyed[member]
    assert "interval" not in conveyed[member]
    assert "interval" in conveyed[director_id]
    # record_pings advanced ONLY the interval-due Director; the stall-check-only
    # member keeps its earlier ping stamp.
    assert (
        broker.get_monitor_config(sid, director_id)["last_ping_at"] == _NOW.isoformat()
    )
    assert broker.get_monitor_config(sid, member)["last_ping_at"] == recent
    # both had a stall-check reason → both baselines committed on the successful wake.
    assert _last_stall_check_at[director_id] == _NOW
    assert _last_stall_check_at[member] == _NOW


def test_monitor_tick__stall_disabled_emits_no_wake_and_no_stall_tag(monkeypatch):
    """With ``monitor_stall_interval == 0`` an interval-not-due member never
    classifies stalled: no ``stall-check`` reason is emitted, and with nothing else
    due there is no wake and no baseline is recorded."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 0)
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")

    recent = _NOW.isoformat()
    broker.record_pings([director_id, member], recent)
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    wakes = _stub_tmux_wakes(monkeypatch, {"%0", "%7", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    assert wakes == []
    assert _last_stall_check_at == {}


def test_monitor_tick__stall_baseline_committed_only_on_successful_wake(monkeypatch):
    """``_last_stall_check_at`` is committed ONLY on a successful wake. A failed
    keystroke leaves the baseline unset, so the member stays stall-check due and the
    next (successful) tick commits it — no silent skip."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")

    # Director interval-due so a wake is attempted; it is also first-tick stall-check due.
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )

    # Tick 1: the wake keystroke FAILS (wake_ok=False) → nothing committed.
    _stub_tmux_wakes(monkeypatch, {"%0", "%7"}, wake_ok=False)
    assert monitor_tick(sid, _NOW) is CONTINUE
    assert _last_stall_check_at == {}
    assert broker.get_monitor_config(sid, director_id)["last_ping_at"] is None

    # Tick 2: the wake SUCCEEDS → the still-due member's baseline is committed now.
    _stub_tmux_wakes(monkeypatch, {"%0", "%7"}, wake_ok=True)
    assert monitor_tick(sid, _NOW) is CONTINUE
    assert _last_stall_check_at[director_id] == _NOW


def test_monitor_tick__first_tick_every_watched_member_stall_check_due(monkeypatch):
    """First-tick semantics: with ``_last_stall_check_at`` empty, EVERY enabled
    watched live member is stall-check due even when all are interval-not-due — the
    dict is not pre-seeded. The wake conveys a ``stall-check`` reason for each, and
    each baseline is committed on the successful wake."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")

    recent = _NOW.isoformat()
    broker.record_pings([director_id, member], recent)
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    wakes = _stub_tmux_wakes(monkeypatch, {"%0", "%7", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    assert len(wakes) == 1
    conveyed = dict(wakes[0][1])
    assert set(conveyed) == {director_id, member}
    assert conveyed[director_id] == ["stall-check"]
    assert conveyed[member] == ["stall-check"]
    # both baselines seeded on the successful wake …
    assert _last_stall_check_at[director_id] == _NOW
    assert _last_stall_check_at[member] == _NOW
    # … and neither ping cadence advanced (stall-check-only → excluded).
    assert broker.get_monitor_config(sid, director_id)["last_ping_at"] == recent
    assert broker.get_monitor_config(sid, member)["last_ping_at"] == recent


def test_run_monitor_loop__clears_stall_check_dict_not_preseeded(monkeypatch):
    """``run_monitor_loop`` clears ``_last_stall_check_at`` at the start of every
    run (the same per-run reset as ``_last_member_status``) and never pre-seeds any
    member — an absent member is exactly what makes tick 1 stall-check due. The clear
    happens before the slot claim, so a claim failure still leaves the dict empty."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    # pre-seed a stale entry; run_monitor_loop must wipe it.
    _last_stall_check_at[999] = _NOW
    monkeypatch.setattr(broker, "claim_monitor_runtime", lambda *a, **k: False)

    with pytest.raises(click.ClickException):
        run_monitor_loop(sid, 5)

    assert _last_stall_check_at == {}
