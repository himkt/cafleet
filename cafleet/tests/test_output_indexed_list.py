"""Tests for ``format_indexed_list``."""

from cafleet.output import format_agent, format_indexed_list, format_task


def test_format_indexed_list__empty_and_non_empty_join_behaviour():
    formatter_calls: list = []

    def formatter(item):
        formatter_calls.append(item)
        return f"FMT-{item}"

    assert (
        format_indexed_list([], formatter, "No widgets found.") == "No widgets found."
    )
    assert formatter_calls == []

    result = format_indexed_list(["a", "b", "c"], formatter, "unused empty msg")
    assert formatter_calls == ["a", "b", "c"]
    assert result == "FMT-a\n\nFMT-b\n\nFMT-c"


def test_format_indexed_list__byte_identical_for_task_and_agent_shapes():
    task = {
        "task_id": 1,
        "context_id": 2,
        "from_agent_id": 11,
        "to_agent_id": 2,
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

    agent = {
        "agent_id": 11,
        "name": "alpha",
        "description": "A test agent",
        "status": "active",
    }
    assert (
        format_indexed_list([agent], format_agent, "No agents found.")
        == "11 alpha active"
    )
