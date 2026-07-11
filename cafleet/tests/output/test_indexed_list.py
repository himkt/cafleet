"""Tests for ``format_indexed_list``."""

from cafleet.output import format_indexed_list, format_member_detail, format_task


def test_format_indexed_list__byte_identical_for_task_and_member_shapes():
    task = {
        "task_id": 1,
        "context_id": 2,
        "from_member_id": 11,
        "to_member_id": 2,
        "type": "unicast",
        "created_at": "2026-05-05T12:00:00.000000+00:00",
        "status_state": "input_required",
        "status_timestamp": "2026-05-05T12:00:00.000000+00:00",
        "origin_task_id": None,
        "text": "hello world",
    }
    expected_task = "[1 | from:11 | 2026-05-05T12:00:00.000000+00:00]\nhello world"
    assert (
        format_indexed_list([task], format_task, "No messages found.") == expected_task
    )

    member = {
        "member_id": 11,
        "name": "alpha",
        "description": "A test member",
        "status": "active",
    }
    assert (
        format_indexed_list([member], format_member_detail, "No members found.")
        == "11 alpha active"
    )
