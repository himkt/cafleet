"""Tests for ``format_indexed_list``.

``format_indexed_list(items, formatter, empty_msg)`` returns the
``empty_msg`` verbatim when ``items`` is empty, and otherwise produces a
newline-joined block of one-based ``[i]`` markers interleaved with each
item's ``formatter(item)`` rendering.
"""

from cafleet.output import format_agent, format_indexed_list, format_task


class TestFormatIndexedList:
    def test_empty_items_returns_empty_msg_verbatim(self):
        formatter_calls = []

        def formatter(item):
            formatter_calls.append(item)
            return "never called"

        result = format_indexed_list([], formatter, "No widgets found.")
        assert result == "No widgets found."
        assert formatter_calls == []

    def test_non_empty_calls_formatter_per_item_with_indexed_prefix(self):
        formatter_calls = []

        def formatter(item):
            formatter_calls.append(item)
            return f"FMT-{item}"

        result = format_indexed_list(["a", "b", "c"], formatter, "unused empty msg")

        assert formatter_calls == ["a", "b", "c"]
        assert result == "[1]\nFMT-a\n[2]\nFMT-b\n[3]\nFMT-c"

    def test_byte_identical_output_for_task_list_shape(self):
        task = {
            "id": "tid-1",
            "status": {"state": "input_required"},
            "metadata": {
                "fromAgentId": "a1",
                "toAgentId": "a2",
                "type": "unicast",
            },
            "artifacts": [{"parts": [{"text": "hello world"}]}],
        }
        result = format_indexed_list([task], format_task, "No messages found.")
        expected = "\n".join(
            [
                "[1]",
                "  id:    tid-1",
                "  state: input_required",
                "  from:  a1",
                "  to:    a2",
                "  type:  unicast",
                "  text:  hello world",
            ]
        )
        assert result == expected

    def test_byte_identical_output_for_agent_list_shape(self):
        agent = {
            "agent_id": "a1",
            "name": "alpha",
            "description": "A test agent",
            "status": "active",
        }
        result = format_indexed_list([agent], format_agent, "No agents found.")
        expected = "\n".join(
            [
                "[1]",
                "  agent_id:    a1",
                "  name:        alpha",
                "  description: A test agent",
                "  status:      active",
            ]
        )
        assert result == expected
