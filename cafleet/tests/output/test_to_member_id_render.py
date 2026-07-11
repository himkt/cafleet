"""Regression tests for the ``to:`` line in the full message render (design 0000118, item 1.2).

The verbose render surfaces ``to:`` when a recipient is present
(``to_member_id`` is not ``None``) and omits it for recipient-less rows such as
broadcast summaries.
"""

from cafleet.output.formatters import format_message


def _full_message(**overrides):
    message = {
        "message_id": 5,
        "owner_member_id": 5,
        "from_member_id": 1,
        "to_member_id": None,
        "type": "broadcast_summary",
        "created_at": "2026-07-04T00:00:00+00:00",
        "status_state": "completed",
        "status_timestamp": "2026-07-04T00:00:00+00:00",
        "origin_message_id": 5,
        "text": "Broadcast sent to 2 recipients",
    }
    message.update(overrides)
    return message


def test_full_render_omits_to_line_when_to_member_id_none():
    rendered = format_message(_full_message(to_member_id=None), full=True)
    assert not any(line.strip().startswith("to:") for line in rendered.splitlines())


def test_full_render_shows_to_line_for_real_recipient():
    rendered = format_message(_full_message(to_member_id=42, type="unicast"), full=True)
    assert "to:    42" in rendered
