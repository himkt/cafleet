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

import cafleet.monitor.loop as monitor_loop_module
from cafleet import broker
from cafleet.config import settings
from cafleet.db.models import Fleet, MemberPlacement, Message, MonitorConfig
from cafleet.monitor.loop import (
    _WAKE_ON_STATUS,
    CONTINUE,
    STOP,
    _flag_native_status_due,
    _flag_stall_check_due,
    _last_member_status,
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


def _annotate_unacked_due(due: list[dict], now: datetime) -> None:
    """Resolve the new helper lazily so pre-implementation tests still collect."""
    monitor_loop_module._annotate_unacked_due(due, now)


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
        self.directors: list[dict] = []
        self.wake_payloads: list[list[dict]] = []

    def list_pane_ids(self):
        return set(self._live)

    def agent_status(self, *, target_pane_id):
        return self._statuses.get(target_pane_id)

    def send_wake_trigger(self, *, target_pane_id, due_members, director):
        assert set(director) == {"member_id", "coding_agent"}
        assert director["coding_agent"] in {"claude", "codex", "opencode"}
        assert all(
            target["coding_agent"] in {"claude", "codex", "opencode"}
            for target in due_members
        )
        self.directors.append(dict(director))
        self.wake_payloads.append(
            [
                {
                    "member_id": target["member_id"],
                    "coding_agent": target["coding_agent"],
                    "wake_reasons": list(target["wake_reasons"]),
                }
                for target in due_members
            ]
        )
        self.wakes.append(
            (
                target_pane_id,
                [t["member_id"] for t in due_members],
                director["member_id"],
            )
        )
        return self._wake_ok


def _native_target(
    member_id,
    pane_id="%9",
    *,
    pane_alive=True,
    name="alice",
    enabled=True,
    coding_agent="claude",
    last_stall_check_at=None,
    oldest_pending_ts=None,
    interval_seconds=720,
    is_director=False,
):
    return {
        "member_id": member_id,
        "pane_id": pane_id,
        "pane_alive": pane_alive,
        "name": name,
        "enabled": enabled,
        "coding_agent": coding_agent,
        "last_stall_check_at": last_stall_check_at,
        "oldest_pending_ts": oldest_pending_ts,
        "interval_seconds": interval_seconds,
        "is_director": is_director,
    }


def _register_watched_member(
    fleet: dict,
    name: str,
    pane_id: str,
    *,
    coding_agent: str = "claude",
) -> int:
    """Register an ordinary member — a watched member enrolled @720."""
    return _register_member(
        fleet["fleet_id"],
        name=name,
        placement=_member_placement(pane_id, coding_agent),
    )["member_id"]


def _iso_ago(seconds: int) -> str:
    return (_NOW - timedelta(seconds=seconds)).isoformat()


def _unacked_target(member_id, *, oldest_pending_ts, interval_seconds=720, **kwargs):
    """A due-row dict for ``_annotate_unacked_due`` carrying its
    own ``interval_seconds`` and the ``oldest_pending_ts`` scan field."""
    return _native_target(
        member_id,
        oldest_pending_ts=oldest_pending_ts,
        interval_seconds=interval_seconds,
        **kwargs,
    )


def _insert_stale_pending(
    s, owner_member_id: int, from_member_id: int, status_timestamp: str
) -> None:
    """Insert an ``input_required`` unicast delivery with a controlled
    ``status_timestamp`` — the age ``oldest_pending_ts`` reads back."""
    s.add(
        Message(
            owner_member_id=owner_member_id,
            from_member_id=from_member_id,
            to_member_id=owner_member_id,
            type="unicast",
            created_at=status_timestamp,
            status_state="input_required",
            status_timestamp=status_timestamp,
            origin_message_id=None,
            text="pending",
        )
    )


def _set_stall_check_at(session, member_id: int, when: str) -> None:
    session.get(MonitorConfig, member_id).last_stall_check_at = when


def _stub_tmux(monkeypatch, live_panes, *, wake_ok=True):
    """Stub pane liveness; capture poll-trigger and wake-trigger keystrokes into
    separate lists. The loop only ever fires ``send_wake_trigger`` (into the
    watcher's pane); ``polls`` is captured to assert the loop never keystrokes a
    watched (Director / member) pane (§4). ``wake_ok`` is the boolean
    ``send_wake_trigger`` returns — pass ``False`` to model a best-effort
    keystroke that was attempted but failed to land. Each ``wakes`` entry is
    ``(target_pane_id, [conveyed due-member ids], director_member_id)`` — the due
    set and the Director id the wake trigger names (§2/§4)."""
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

    def fake_wake(self, *, target_pane_id, due_members, director):
        assert set(director) == {"member_id", "coding_agent"}
        assert director["coding_agent"] in {"claude", "codex", "opencode"}
        assert all(
            target["coding_agent"] in {"claude", "codex", "opencode"}
            for target in due_members
        )
        wakes.append(
            (
                target_pane_id,
                [t["member_id"] for t in due_members],
                director["member_id"],
            )
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

    def fake_wake(self, *, target_pane_id, due_members, director):
        assert set(director) == {"member_id", "coding_agent"}
        assert director["coding_agent"] in {"claude", "codex", "opencode"}
        assert all(
            target["coding_agent"] in {"claude", "codex", "opencode"}
            for target in due_members
        )
        wakes.append(
            (
                target_pane_id,
                [
                    (t["member_id"], list(t.get("wake_reasons", [])))
                    for t in due_members
                ],
                director["member_id"],
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


def test_flag_native_status_due__already_due_appends_status_reason():
    """A native done transition is unioned onto an already-due row."""
    mux = _FakeStateMux({"%9"}, {"%9": "done"})
    target = _native_target(5)
    target["wake_reasons"] = ["interval"]
    due = [target]
    pending = _flag_native_status_due(mux, [target], due)
    assert [t["member_id"] for t in due] == [5]
    assert target["wake_reasons"] == ["interval", "status:done"]
    assert pending == {5: "done"}


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
    # The helper is read-only; persistence is gated on a successful wake.
    assert targets[0]["last_stall_check_at"] is None


def test_flag_stall_check_due__not_due_before_interval(monkeypatch):
    """Within one interval of the last stall-check the member is not re-flagged."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    targets = [
        _native_target(
            5,
            last_stall_check_at=(_NOW - timedelta(seconds=100)).isoformat(),
        )
    ]
    due: list[dict] = []
    _flag_stall_check_due(targets, due, _NOW)
    assert due == []


def test_flag_stall_check_due__due_once_interval_elapsed(monkeypatch):
    """At exactly one interval since the last stall-check the member is due again."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    targets = [
        _native_target(
            5,
            last_stall_check_at=(_NOW - timedelta(seconds=240)).isoformat(),
        )
    ]
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
    # Both stall dispatch timestamps commit durably on the successful wake.
    assert (
        broker.get_monitor_config(sid, director_id)["last_stall_check_at"]
        == _NOW.isoformat()
    )
    assert (
        broker.get_monitor_config(sid, member)["last_stall_check_at"]
        == _NOW.isoformat()
    )


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
    assert broker.get_monitor_config(sid, director_id)["last_stall_check_at"] is None
    assert broker.get_monitor_config(sid, member)["last_stall_check_at"] is None


def test_monitor_tick__stall_dispatch_committed_only_on_successful_wake(monkeypatch):
    """The durable stall dispatch timestamp is gated on a successful wake."""
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
    assert broker.get_monitor_config(sid, director_id)["last_stall_check_at"] is None
    assert broker.get_monitor_config(sid, director_id)["last_ping_at"] is None

    # Tick 2: the wake succeeds, committing the same dispatch time durably.
    _stub_tmux_wakes(monkeypatch, {"%0", "%7"}, wake_ok=True)
    assert monitor_tick(sid, _NOW) is CONTINUE
    assert (
        broker.get_monitor_config(sid, director_id)["last_stall_check_at"]
        == _NOW.isoformat()
    )


def test_monitor_tick__null_dispatch_every_watched_member_stall_check_due(monkeypatch):
    """With durable ``last_stall_check_at`` null, every enabled
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
    # Both durable dispatch timestamps commit on the successful wake …
    assert (
        broker.get_monitor_config(sid, director_id)["last_stall_check_at"]
        == _NOW.isoformat()
    )
    assert (
        broker.get_monitor_config(sid, member)["last_stall_check_at"]
        == _NOW.isoformat()
    )
    # … and neither ping cadence advanced (stall-check-only → excluded).
    assert broker.get_monitor_config(sid, director_id)["last_ping_at"] == recent
    assert broker.get_monitor_config(sid, member)["last_ping_at"] == recent


def test_run_monitor_loop__does_not_reset_durable_stall_dispatch(
    broker_session, monkeypatch
):
    """A process restart leaves the persisted stall cadence untouched."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    persisted = (_NOW - timedelta(seconds=30)).isoformat()
    with broker_session() as session:
        session.get(MonitorConfig, director_id).last_stall_check_at = persisted
        session.commit()
    monkeypatch.setattr(broker, "claim_monitor_runtime", lambda *a, **k: False)

    with pytest.raises(click.ClickException):
        run_monitor_loop(sid, 5)

    assert (
        broker.get_monitor_config(sid, director_id)["last_stall_check_at"] == persisted
    )


# --- startup line (the ready-handshake backing) ------------------------------
#
# ``run_monitor_loop`` emits ``monitor loop started (fleet <fleet_id>, tick
# <tick>s, pid <pid>)`` to stdout immediately after claiming the runtime row and
# before the first tick. The monitoring member confirms this line in its
# background-task output before sending ``ready: monitor live``.


def test_run_monitor_loop__emits_startup_line_after_claim_before_first_tick(
    capsys, monkeypatch
):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]

    def fake_tick(fleet_id, now):
        click.echo("tick-ran")
        return STOP

    monkeypatch.setattr(monitor_loop_module, "monitor_tick", fake_tick)

    run_monitor_loop(sid, 5)

    lines = capsys.readouterr().out.splitlines()
    assert lines[0] == f"monitor loop started (fleet {sid}, tick 5s, pid {os.getpid()})"
    assert lines[1] == "tick-ran"


def test_run_monitor_loop__no_startup_line_when_claim_refused(capsys, monkeypatch):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    monkeypatch.setattr(broker, "claim_monitor_runtime", lambda *_a, **_kw: False)

    with pytest.raises(click.ClickException, match="already running"):
        run_monitor_loop(sid, 5)

    assert "monitor loop started" not in capsys.readouterr().out


# --- unacked-delivery annotation -------------------------------------------
#
# Only rows already due for interval, durable stall-check, or native done can
# receive the ``unacked`` hint. It is appended last and has no cadence of its own.


def test_annotate_unacked_due__appends_at_exactly_one_interval():
    target = _unacked_target(5, oldest_pending_ts=_iso_ago(720))
    target["wake_reasons"] = ["interval"]
    due = [target]

    _annotate_unacked_due(due, _NOW)

    assert target["wake_reasons"] == ["interval", "unacked"]


def test_annotate_unacked_due__not_yet_stale_skipped():
    """A pending delivery younger than one interval never flags — the normal
    deliver-then-ack cycle produces no wakes."""
    target = _unacked_target(5, oldest_pending_ts=_iso_ago(719))
    target["wake_reasons"] = ["stall-check"]
    _annotate_unacked_due([target], _NOW)
    assert target["wake_reasons"] == ["stall-check"]


def test_annotate_unacked_due__no_pending_skipped():
    """``oldest_pending_ts is None`` (no pending delivery) never flags."""
    target = _unacked_target(5, oldest_pending_ts=None)
    target["wake_reasons"] = ["status:done"]
    _annotate_unacked_due([target], _NOW)
    assert target["wake_reasons"] == ["status:done"]


def test_annotate_unacked_due__empty_due_set_stays_empty():
    """A stale delivery cannot add a row because the helper receives due rows only."""
    due: list[dict] = []
    _annotate_unacked_due(due, _NOW)
    assert due == []


def test_annotate_unacked_due__existing_hint_is_not_duplicated():
    target = _unacked_target(5, oldest_pending_ts=_iso_ago(1500))
    target["wake_reasons"] = ["interval", "unacked"]
    _annotate_unacked_due([target], _NOW)
    assert target["wake_reasons"] == ["interval", "unacked"]


def test_annotate_unacked_due__uses_members_own_interval():
    """The staleness threshold is each member's OWN ``interval_seconds`` from the
    same scan row: the same 300 s-old delivery is stale for a 180 s member and
    not for a 720 s member."""
    due = [
        _unacked_target(5, oldest_pending_ts=_iso_ago(300), interval_seconds=180),
        _unacked_target(
            6, oldest_pending_ts=_iso_ago(300), interval_seconds=720, pane_id="%10"
        ),
    ]
    for target in due:
        target["wake_reasons"] = ["stall-check"]
    _annotate_unacked_due(due, _NOW)
    assert due[0]["wake_reasons"] == ["stall-check", "unacked"]
    assert due[1]["wake_reasons"] == ["stall-check"]


def test_annotate_unacked_due__appends_reason_last_to_already_due_member():
    """An unacked-due member already in the due set (e.g. interval-due) is not
    appended twice; ``unacked`` is UNIONED onto its existing reasons."""
    target = _unacked_target(5, oldest_pending_ts=_iso_ago(800))
    target["wake_reasons"] = ["interval"]
    due = [target]
    _annotate_unacked_due(due, _NOW)
    assert [t["member_id"] for t in due] == [5]
    assert target["wake_reasons"] == ["interval", "unacked"]


def test_annotate_unacked_due__preserves_due_row_order():
    first = _unacked_target(5, oldest_pending_ts=_iso_ago(800))
    second = _unacked_target(6, oldest_pending_ts=None, pane_id="%10")
    first["wake_reasons"] = ["interval"]
    second["wake_reasons"] = ["status:done"]
    due = [first, second]
    _annotate_unacked_due(due, _NOW)
    assert [target["member_id"] for target in due] == [5, 6]


def test_annotate_unacked_due__does_not_change_non_unacked_reasons():
    target = _unacked_target(5, oldest_pending_ts=None)
    target["wake_reasons"] = ["interval", "stall-check", "status:done"]
    _annotate_unacked_due([target], _NOW)
    assert target["wake_reasons"] == ["interval", "stall-check", "status:done"]


def test_monitor_tick__stale_unacked_without_normal_trigger_does_not_wake(
    broker_session, monkeypatch
):
    """A stale delivery alone cannot create a due member or watcher wake."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")

    # both interval-not-due; the member's delivery is 800 s old (≥ its 720 s)
    recent = _NOW.isoformat()
    broker.record_pings([director_id, member], recent)
    with broker_session() as s:
        _insert_stale_pending(s, member, director_id, _iso_ago(800))
        s.commit()
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    wakes = _stub_tmux_wakes(monkeypatch, {"%0", "%7", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    assert wakes == []
    assert broker.get_monitor_config(sid, member)["last_ping_at"] == recent
    assert broker.get_monitor_config(sid, director_id)["last_ping_at"] == recent


def test_monitor_tick__stale_hint_reappears_on_each_normal_due_wake(
    broker_session, monkeypatch
):
    """No re-fire map exists: normal wakes carry the stale hint until ACK."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")

    recent = (_NOW - timedelta(seconds=720)).isoformat()
    broker.record_pings([director_id], _NOW.isoformat())
    broker.record_pings([member], recent)
    with broker_session() as s:
        _insert_stale_pending(s, member, director_id, _iso_ago(800))
        s.commit()
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )

    wakes1 = _stub_tmux_wakes(monkeypatch, {"%0", "%7", "%9"})
    assert monitor_tick(sid, _NOW) is CONTINUE
    assert len(wakes1) == 1
    assert dict(wakes1[0][1]) == {member: ["interval", "unacked"]}

    broker.record_pings([member], recent)
    wakes2 = _stub_tmux_wakes(monkeypatch, {"%0", "%7", "%9"}, wake_ok=True)
    assert monitor_tick(sid, _NOW) is CONTINUE
    assert len(wakes2) == 1
    assert dict(wakes2[0][1]) == {member: ["interval", "unacked"]}


def test_monitor_tick__reason_order_interval_stall_check_unacked(
    broker_session, monkeypatch
):
    """A member due on all three triggers carries ``wake_reasons`` in the call
    order ``interval``, ``stall-check``, ``unacked`` because annotation runs
    after every trigger, and both real cadences commit on one successful wake."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")

    # never pinged → interval-due; absent stall baseline → stall-check due;
    # 800 s-old delivery (≥ 720 s) → unacked due.
    with broker_session() as s:
        _insert_stale_pending(s, member, director_id, _iso_ago(800))
        s.commit()
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    wakes = _stub_tmux_wakes(monkeypatch, {"%0", "%7", "%9"})

    result = monitor_tick(sid, _NOW)

    assert result is CONTINUE
    assert len(wakes) == 1
    conveyed = dict(wakes[0][1])
    assert conveyed[member] == ["interval", "stall-check", "unacked"]
    assert conveyed[director_id] == ["interval", "stall-check"]
    # The interval and durable stall cadences commit on the one successful wake.
    assert broker.get_monitor_config(sid, member)["last_ping_at"] == _NOW.isoformat()
    assert (
        broker.get_monitor_config(sid, member)["last_stall_check_at"]
        == _NOW.isoformat()
    )


def test_monitor_tick__reason_order_all_triggers_unacked_last(
    broker_session, monkeypatch
):
    """Interval, durable stall, native done, and stale-delivery context are
    unioned onto one member in construction order and still produce one wake."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9")

    broker.record_pings([director_id], _NOW.isoformat())
    with broker_session() as session:
        director_config = session.get(MonitorConfig, director_id)
        director_config.last_stall_check_at = _NOW.isoformat()
        _insert_stale_pending(session, member, director_id, _iso_ago(800))
        session.commit()
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    mux = _FakeStateMux(
        {"%0", "%7", "%9"},
        {"%0": "idle", "%9": "done"},
    )
    monkeypatch.setattr("cafleet.monitor.loop.resolve_multiplexer", lambda: mux)

    assert monitor_tick(sid, _NOW) is CONTINUE

    assert len(mux.wake_payloads) == 1
    assert mux.wake_payloads[0] == [
        {
            "member_id": member,
            "coding_agent": "claude",
            "wake_reasons": [
                "interval",
                "stall-check",
                "status:done",
                "unacked",
            ],
        }
    ]


def test_monitor_tick__mixed_backend_metadata_reaches_one_wake(monkeypatch):
    """Every due row uses its own backend while the Director has an independent
    descriptor; mixed backends remain one synchronized watcher wake."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    codex_member = _register_watched_member(
        fleet, "codex-member", "%9", coding_agent="codex"
    )
    opencode_member = _register_watched_member(
        fleet, "opencode-member", "%10", coding_agent="opencode"
    )
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    mux = _FakeStateMux(
        {"%0", "%7", "%9", "%10"},
        {"%0": "idle", "%9": "idle", "%10": "idle"},
    )
    monkeypatch.setattr("cafleet.monitor.loop.resolve_multiplexer", lambda: mux)

    assert monitor_tick(sid, _NOW) is CONTINUE

    assert mux.directors == [{"member_id": director_id, "coding_agent": "claude"}]
    assert len(mux.wake_payloads) == 1
    assert {row["member_id"]: row["coding_agent"] for row in mux.wake_payloads[0]} == {
        director_id: "claude",
        codex_member: "codex",
        opencode_member: "opencode",
    }


def test_monitor_tick__invalid_coding_agent_fails_closed_without_cadence_commit(
    broker_session, monkeypatch
):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_watched_member(fleet, "alice", "%9", coding_agent="codex")
    broker.record_pings([director_id], _NOW.isoformat())
    with broker_session() as session:
        session.get(MemberPlacement, member).coding_agent = "not-registered"
        session.commit()
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    attempted_wakes: list[bool] = []
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.list_pane_ids",
        lambda self: {"%0", "%7", "%9"},
        raising=False,
    )

    def fake_wake(self, **kwargs):
        attempted_wakes.append(True)
        return True

    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.send_wake_trigger",
        fake_wake,
        raising=False,
    )

    with pytest.raises(click.ClickException, match="coding.agent|coding_agent"):
        monitor_tick(sid, _NOW)

    assert attempted_wakes == []
    config = broker.get_monitor_config(sid, member)
    assert config["last_ping_at"] is None
    assert config["last_stall_check_at"] is None


def test_monitor_tick__reconciles_nonlive_members_before_due_filtering(
    broker_session, monkeypatch
):
    """Dead, placement-pending, and disabled targets are cleaned in one scan:
    lifecycle reconciliation clears only ``last_stall_check_at``, leaving the
    schedule fields untouched."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    dead = _register_watched_member(fleet, "dead", "%9")
    paneless = _register_member(
        sid,
        name="paneless",
        placement=_member_placement(None),
    )["member_id"]
    disabled = _register_watched_member(fleet, "disabled", "%11")
    broker.record_pings([director_id, dead, paneless, disabled], _NOW.isoformat())
    checked = (_NOW - timedelta(seconds=60)).isoformat()
    with broker_session() as session:
        _set_stall_check_at(session, dead, checked)
        _set_stall_check_at(session, paneless, checked)
        _set_stall_check_at(session, disabled, checked)
        session.get(MonitorConfig, disabled).enabled = 0
        session.commit()
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )
    _, wakes = _stub_tmux(monkeypatch, {"%0", "%7"})

    assert monitor_tick(sid, _NOW) is CONTINUE
    assert wakes == []

    for member_id in (dead, paneless, disabled):
        config = broker.get_monitor_config(sid, member_id)
        assert config["last_stall_check_at"] is None
        # only the stall-check cadence is cleared — the schedule survives
        assert config["last_ping_at"] == _NOW.isoformat()
        assert config["interval_seconds"] == 720


def test_monitor_tick__live_rebind_reseeds_stall_dispatch_after_cleanup(
    broker_session, monkeypatch
):
    """A pending placement clears the dispatch timestamp; once rebound live,
    the next scan seeds a fresh durable stall-check cadence."""
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    member = _register_member(
        sid,
        name="recovering",
        placement=_member_placement(None, "codex"),
    )["member_id"]
    broker.record_pings([director_id, member], _NOW.isoformat())
    with broker_session() as session:
        _set_stall_check_at(session, member, (_NOW - timedelta(seconds=60)).isoformat())
        session.get(MonitorConfig, director_id).last_stall_check_at = _NOW.isoformat()
        session.commit()
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )

    _, first_wakes = _stub_tmux(monkeypatch, {"%0", "%7"})
    assert monitor_tick(sid, _NOW) is CONTINUE
    assert first_wakes == []
    cleaned = broker.get_monitor_config(sid, member)
    assert cleaned["last_stall_check_at"] is None

    broker.update_placement_pane_id(member, "%9")
    recovery_wakes = _stub_tmux_wakes(monkeypatch, {"%0", "%7", "%9"})
    later = _NOW + timedelta(seconds=1)
    assert monitor_tick(sid, later) is CONTINUE
    assert dict(recovery_wakes[0][1]) == {member: ["stall-check"]}
    assert (
        broker.get_monitor_config(sid, member)["last_stall_check_at"]
        == later.isoformat()
    )


def test_monitor_tick__immediate_restart_honors_durable_stall_dispatch(monkeypatch):
    monkeypatch.setattr(settings, "monitor_stall_interval", 240)
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    _register_monitoring_member(fleet, "watcher", "%7")
    broker.record_pings([director_id], _NOW.isoformat())
    broker.claim_monitor_runtime(
        sid, os.getpid(), 5, (_NOW - timedelta(seconds=30)).isoformat()
    )

    first_wakes = _stub_tmux_wakes(monkeypatch, {"%0", "%7"})
    assert monitor_tick(sid, _NOW) is CONTINUE
    assert dict(first_wakes[0][1]) == {director_id: ["stall-check"]}
    assert (
        broker.get_monitor_config(sid, director_id)["last_stall_check_at"]
        == _NOW.isoformat()
    )

    # The only process-local transition cache may reset on a new monitor process.
    _last_member_status.clear()
    second_wakes = _stub_tmux_wakes(monkeypatch, {"%0", "%7"})
    assert monitor_tick(sid, _NOW + timedelta(seconds=1)) is CONTINUE
    assert second_wakes == []


def test_loop_module_has_no_process_local_stall_or_unacked_maps():
    import cafleet.monitor.loop as loop

    assert not hasattr(loop, "_last_stall_check_at")
    assert not hasattr(loop, "_last_unacked_wake_at")
