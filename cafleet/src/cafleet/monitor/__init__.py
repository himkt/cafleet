"""The ``cafleet monitor`` policy package — tunables (§10).

Three tunables are re-exported from ``broker/monitor.py`` (the schedule
default and the staleness window the broker's liveness check computes with) so
they have a single home in the data-access layer; one is monitor-loop-local
(the scan-tick cadence).
"""

from cafleet.broker.monitor import (
    DEFAULT_PING_INTERVAL_SECONDS,
    MONITOR_STALE_FACTOR,
    MONITOR_STALE_FLOOR_SECONDS,
)

DEFAULT_TICK_SECONDS = 5

__all__ = [
    "DEFAULT_PING_INTERVAL_SECONDS",
    "DEFAULT_TICK_SECONDS",
    "MONITOR_STALE_FACTOR",
    "MONITOR_STALE_FLOOR_SECONDS",
]
