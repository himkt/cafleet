"""Exact text layout for the ``monitor status`` watched-member table.

Pins the header, separator, and row rendering: the ``unacked`` column sits
after ``pending`` (``:<7``-padded now that it is no longer last) and renders
the pending age via ``_format_ping_age`` — ``<age>s ago`` with a pending
delivery, ``-`` without one.
"""

from cafleet.output.formatters import format_monitor_status


def test_format_monitor_status__exact_table_with_and_without_pending_age():
    rendered = format_monitor_status(
        {
            "runtime": {"running": False},
            "members": [
                {
                    "member_id": 12,
                    "name": "alice",
                    "role": "member",
                    "interval_seconds": 720,
                    "last_ping_age_seconds": 63,
                    "enabled": True,
                    "pending_count": 2,
                    "oldest_pending_age_seconds": 811,
                },
                {
                    "member_id": 7,
                    "name": "bob",
                    "role": "director",
                    "interval_seconds": 180,
                    "last_ping_age_seconds": None,
                    "enabled": True,
                    "pending_count": 0,
                    "oldest_pending_age_seconds": None,
                },
            ],
        }
    )
    assert rendered == (
        "monitor: stopped\n"
        "  member_id  name         role      interval  last_ping  enabled  pending  unacked\n"
        "  ---------  -----------  --------  --------  ---------  -------  -------  -------\n"
        "  12         alice        member    720s      63s ago    yes      2        811s ago\n"
        "  7          bob          director  180s      -          yes      0        -"
    )
