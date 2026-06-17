"""Unit tests for the pure ``should_ping`` policy (§4).

``should_ping(target, now)`` is a pure function of a scan-row dict and a
tz-aware ``now``, so these tests need neither tmux nor the DB. The scan rows are
the watched agents — the root Director (180 s) and every ordinary member
(720 s); the monitoring member is the unenrolled watcher and never appears here.
``is_director`` is retained for ``monitor status`` labeling but ``should_ping``
is role-agnostic: enabled + live pane + interval elapsed ⇒ due.
"""

from datetime import UTC, datetime, timedelta

from cafleet.monitor.loop import should_ping

_NOW = datetime(2026, 6, 13, 12, 0, 0, tzinfo=UTC)


def _target(**overrides):
    """A watched ordinary-member target (@720). ``should_ping`` ignores
    ``is_director``; the role flag is overridden in the Director-interval case."""
    base = {
        "agent_id": 1,
        "name": "x",
        "is_director": False,
        "pane_id": "%1",
        "pane_alive": True,
        "interval_seconds": 720,
        "last_ping_at": None,
        "enabled": True,
        "pending_count": 0,
    }
    base.update(overrides)
    return base


def test_should_ping__due_when_never_pinged():
    # a never-pinged watched agent is due even with an empty inbox
    assert should_ping(_target(), _NOW) is True


def test_should_ping__due_after_interval_with_empty_inbox():
    target = _target(
        pending_count=0,
        last_ping_at=(_NOW - timedelta(seconds=800)).isoformat(),
    )
    assert should_ping(target, _NOW) is True


def test_should_ping__pinged_regardless_of_pending_count():
    # pending_count never gates the decision (R2): a zero-inbox watched agent is
    # still pinged once due
    assert should_ping(_target(pending_count=0), _NOW) is True
    assert should_ping(_target(pending_count=5), _NOW) is True


def test_should_ping__not_due_skipped():
    # the interval gate still applies — a not-yet-due agent is skipped
    target = _target(last_ping_at=(_NOW - timedelta(seconds=30)).isoformat())
    assert should_ping(target, _NOW) is False


def test_should_ping__director_interval_respected():
    # the watched Director is @180: not due at 120 s elapsed, due at 200 s
    not_due = _target(
        is_director=True,
        interval_seconds=180,
        last_ping_at=(_NOW - timedelta(seconds=120)).isoformat(),
    )
    assert should_ping(not_due, _NOW) is False
    due = _target(
        is_director=True,
        interval_seconds=180,
        last_ping_at=(_NOW - timedelta(seconds=200)).isoformat(),
    )
    assert should_ping(due, _NOW) is True


def test_should_ping__disabled_skipped():
    # disable wins over everything, including a due target with pending work
    target = _target(pending_count=3, enabled=False)
    assert should_ping(target, _NOW) is False


def test_should_ping__missing_pane_skipped():
    target = _target(pending_count=3, pane_id=None)
    assert should_ping(target, _NOW) is False


def test_should_ping__dead_pane_skipped():
    target = _target(pending_count=3, pane_alive=False)
    assert should_ping(target, _NOW) is False


def test_should_ping__last_ping_none_due_immediately():
    # NULL last_ping_at means never pinged ⇒ due now
    assert should_ping(_target(last_ping_at=None), _NOW) is True
