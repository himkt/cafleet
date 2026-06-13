"""The ``cafleet monitor`` process/policy package — tunables (§10).

Three tunables are re-exported from ``broker/monitor.py`` (the schedule
default and the staleness window the broker's liveness check computes with) so
they have a single home in the data-access layer; two are
monitor-process-local (the scan-tick cadence and the stop-signal timeout).
"""

from cafleet.broker.monitor import (
    DEFAULT_PING_INTERVAL_SECONDS,
    MONITOR_STALE_FACTOR,
    MONITOR_STALE_FLOOR_SECONDS,
)

DEFAULT_TICK_SECONDS = 5
MONITOR_STOP_TIMEOUT = 5

__all__ = [
    "DEFAULT_PING_INTERVAL_SECONDS",
    "DEFAULT_TICK_SECONDS",
    "MONITOR_STALE_FACTOR",
    "MONITOR_STALE_FLOOR_SECONDS",
    "MONITOR_STOP_TIMEOUT",
]
