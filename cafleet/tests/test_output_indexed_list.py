"""Tests for ``format_indexed_list``.

Surface 17 dropped the legacy ``[1]`` / ``[2]`` index markers. Items are now
joined by a single blank line (``\\n\\n``) — the per-item formatter output is
emitted verbatim with no prefix.
"""

from cafleet.output import format_agent, format_indexed_list, format_task


def test_format_indexed_list__empty_items_returns_empty_msg_verbatim():
    formatter_calls = []

    def formatter(item):
        formatter_calls.append(item)
        return "never called"

    result = format_indexed_list([], formatter, "No widgets found.")
    assert result == "No widgets found."
    assert formatter_calls == []


def test_format_indexed_list__non_empty_joins_formatter_outputs_with_blank_line():
    formatter_calls = []

    def formatter(item):
        formatter_calls.append(item)
        return f"FMT-{item}"

    result = format_indexed_list(["a", "b", "c"], formatter, "unused empty msg")

    assert formatter_calls == ["a", "b", "c"]
    assert result == "FMT-a\n\nFMT-b\n\nFMT-c"


def test_format_indexed_list__byte_identical_output_for_task_list_shape():
    """Default format_task is the post-Surface-1 2-line compact render."""
    task = {
        "task_id": "tid-1",
        "context_id": "a2",
        "from_agent_id": "a1",
        "to_agent_id": "a2",
        "type": "unicast",
        "created_at": "2026-05-05T12:00:00.000000+00:00",
        "status_state": "input_required",
        "status_timestamp": "2026-05-05T12:00:00.000000+00:00",
        "origin_task_id": None,
        "text": "hello world",
    }
    result = format_indexed_list([task], format_task, "No messages found.")
    expected = "\n".join(
        [
            "[tid-1 | from:a1 | 2026-05-05T12:00:00.000000+00:00]",
            "hello world",
        ]
    )
    assert result == expected


def test_format_indexed_list__byte_identical_output_for_agent_list_shape():
    """Default format_agent is the post-Surface-3 1-line compact render."""
    agent = {
        "agent_id": "a1",
        "name": "alpha",
        "description": "A test agent",
        "status": "active",
    }
    result = format_indexed_list([agent], format_agent, "No agents found.")
    assert result == "a1 alpha active"
