"""The ``cafleet.monitor`` package exposes all five tunables (§10).

Four are re-exported from ``broker/monitor.py`` (the two role-based schedule
defaults and the staleness window the broker's liveness check computes with);
one is monitor-process-local (the scan-tick cadence).
"""

from cafleet import monitor


def test_monitor_package_exposes_all_constants():
    assert monitor.DIRECTOR_PING_INTERVAL_SECONDS == 180
    assert monitor.MEMBER_PING_INTERVAL_SECONDS == 720
    assert monitor.DEFAULT_TICK_SECONDS == 5
    assert monitor.MONITOR_STALE_FACTOR == 3
    assert monitor.MONITOR_STALE_FLOOR_SECONDS == 15
