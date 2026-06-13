"""Unit tests for the pure ``should_ping`` policy (§4).

``should_ping(target, now)`` is a pure function of a scan-row dict and a
tz-aware ``now``, so these tests need neither tmux nor the DB.
"""

from datetime import UTC, datetime, timedelta

from cafleet.monitor.loop import should_ping

_NOW = datetime(2026, 6, 13, 12, 0, 0, tzinfo=UTC)


def _target(**overrides):
    base = {
        "agent_id": 1,
        "name": "x",
        "is_director": False,
        "pane_id": "%1",
        "pane_alive": True,
        "interval_seconds": 60,
        "last_ping_at": None,
        "enabled": True,
        "pending_count": 0,
    }
    base.update(overrides)
    return base


def test_should_ping__director_unconditional_when_never_pinged():
    # director pings even with an empty inbox
    assert should_ping(_target(is_director=True), _NOW) is True


def test_should_ping__director_pings_after_interval_with_empty_inbox():
    target = _target(
        is_director=True,
        pending_count=0,
        last_ping_at=(_NOW - timedelta(seconds=120)).isoformat(),
    )
    assert should_ping(target, _NOW) is True


def test_should_ping__member_with_pending_due():
    assert should_ping(_target(pending_count=1), _NOW) is True


def test_should_ping__member_without_pending_pinged():
    # R2: members are pinged unconditionally once due, regardless of pending_count
    assert should_ping(_target(pending_count=0), _NOW) is True


def test_should_ping__member_not_due_skipped():
    # the interval gate still applies — a not-yet-due agent is skipped
    target = _target(last_ping_at=(_NOW - timedelta(seconds=30)).isoformat())
    assert should_ping(target, _NOW) is False


def test_should_ping__director_not_due_skipped():
    target = _target(
        is_director=True,
        last_ping_at=(_NOW - timedelta(seconds=30)).isoformat(),
    )
    assert should_ping(target, _NOW) is False


def test_should_ping__disabled_skipped():
    # disable wins over everything, including a due director with pending work
    target = _target(is_director=True, pending_count=3, enabled=False)
    assert should_ping(target, _NOW) is False


def test_should_ping__missing_pane_skipped():
    target = _target(is_director=True, pending_count=3, pane_id=None)
    assert should_ping(target, _NOW) is False


def test_should_ping__dead_pane_skipped():
    target = _target(is_director=True, pending_count=3, pane_alive=False)
    assert should_ping(target, _NOW) is False


def test_should_ping__last_ping_none_due_immediately():
    # NULL last_ping_at means never pinged ⇒ due now
    assert should_ping(_target(pending_count=2, last_ping_at=None), _NOW) is True
