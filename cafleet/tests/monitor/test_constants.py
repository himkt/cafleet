"""The ``cafleet.monitor`` package exposes all five tunables (§10).

Three are re-exported from ``broker/monitor.py`` (the schedule defaults and
the staleness window) and two are monitor-process-local (tick cadence + stop
timeout).
"""

from cafleet import monitor


def test_monitor_package_exposes_all_constants():
    assert monitor.DEFAULT_PING_INTERVAL_SECONDS == 60
    assert monitor.DEFAULT_TICK_SECONDS == 5
    assert monitor.MONITOR_STALE_FACTOR == 3
    assert monitor.MONITOR_STALE_FLOOR_SECONDS == 15
    assert monitor.MONITOR_STOP_TIMEOUT == 5
