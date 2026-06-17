"""The ``cafleet monitor`` policy package — tunables (§10).

Four tunables are re-exported from ``broker/monitor.py`` (the two role-based
schedule defaults and the staleness window the broker's liveness check computes
with) so they have a single home in the data-access layer; one is
monitor-loop-local (the scan-tick cadence).
"""

from cafleet.broker.monitor import (
    DIRECTOR_PING_INTERVAL_SECONDS,
    MEMBER_PING_INTERVAL_SECONDS,
    MONITOR_STALE_FACTOR,
    MONITOR_STALE_FLOOR_SECONDS,
)

DEFAULT_TICK_SECONDS = 5

__all__ = [
    "DIRECTOR_PING_INTERVAL_SECONDS",
    "MEMBER_PING_INTERVAL_SECONDS",
    "DEFAULT_TICK_SECONDS",
    "MONITOR_STALE_FACTOR",
    "MONITOR_STALE_FLOOR_SECONDS",
]
