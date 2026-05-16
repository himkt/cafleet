"""Tests for ``format_indexed_list``.

Per principle (ii) of design 0000061: per-API parametrized "shape + behaviour" pairs.
"""

from cafleet.output import format_agent, format_indexed_list, format_task


def test_format_indexed_list__empty_and_non_empty_join_behaviour():
    formatter_calls: list = []

    def formatter(item):
        formatter_calls.append(item)
        return f"FMT-{item}"

    assert format_indexed_list([], formatter, "No widgets found.") == "No widgets found."
    assert formatter_calls == []

    result = format_indexed_list(["a", "b", "c"], formatter, "unused empty msg")
    assert formatter_calls == ["a", "b", "c"]
    assert result == "FMT-a\n\nFMT-b\n\nFMT-c"


def test_format_indexed_list__byte_identical_for_task_and_agent_shapes():
    task = {
        "task_id": "tid-1", "context_id": "a2",
        "from_agent_id": "a1", "to_agent_id": "a2",
        "type": "unicast",
        "created_at": "2026-05-05T12:00:00.000000+00:00",
        "status_state": "input_required",
        "status_timestamp": "2026-05-05T12:00:00.000000+00:00",
        "origin_task_id": None,
        "text": "hello world",
    }
    expected_task = (
        "[tid-1 | from:a1 | 2026-05-05T12:00:00.000000+00:00]\n"
        "hello world"
    )
    assert format_indexed_list([task], format_task, "No messages found.") == expected_task

    agent = {
        "agent_id": "a1", "name": "alpha",
        "description": "A test agent", "status": "active",
    }
    assert format_indexed_list([agent], format_agent, "No agents found.") == "a1 alpha active"
