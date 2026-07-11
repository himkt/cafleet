"""Regression tests: ping-age placeholder is ASCII '-' (design 0000118, item 4.1).

``_format_ping_age(None)`` renders the never-pinged placeholder as ASCII ``-``
(matching the sibling ``_format_idle``), not the EM DASH ``—`` (U+2014). The
full ``monitor status`` text render therefore contains no U+2014.
"""

from cafleet.output.formatters import _format_ping_age, format_monitor_status

_EM_DASH = "—"


def test_format_ping_age_none_returns_ascii_hyphen():
    result = _format_ping_age(None)
    assert result == "-"
    assert _EM_DASH not in result


def test_format_ping_age_real_age_unchanged():
    assert _format_ping_age(42) == "42s ago"


def _never_pinged_payload():
    return {
        "runtime": {"running": False},
        "members": [
            {
                "member_id": 7,
                "name": "member-a",
                "role": "member",
                "interval_seconds": 720,
                "last_ping_age_seconds": None,
                "enabled": True,
                "pending_count": 0,
            }
        ],
    }


def test_format_monitor_status_no_em_dash_for_never_pinged_member():
    rendered = format_monitor_status(_never_pinged_payload())
    # The never-pinged member's row is rendered (guards against a vacuous check).
    assert "member-a" in rendered
    assert _EM_DASH not in rendered
