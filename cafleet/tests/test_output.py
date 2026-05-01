"""Tests for ``cafleet.output`` formatting helpers."""

from cafleet.output import (
    format_member,
    format_member_list,
    truncate_task_text,
    truncate_text,
)


def _member(**placement_overrides) -> dict:
    placement = {
        "tmux_pane_id": "%7",
        "tmux_window_id": "@3",
        "coding_agent": "claude",
    }
    placement.update(placement_overrides)
    return {
        "agent_id": "agent-001",
        "name": "Claude-B",
        "placement": placement,
    }


def _list_entry(*, agent_id: str, name: str, coding_agent: str, pane_id: str) -> dict:
    return {
        "agent_id": agent_id,
        "name": name,
        "status": "active",
        "registered_at": "2026-04-12T10:15:00Z",
        "placement": {
            "director_agent_id": "dir-001",
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": pane_id,
            "coding_agent": coding_agent,
            "created_at": "2026-04-12T10:15:00Z",
        },
    }


class TestFormatMember:
    def test_includes_backend_line(self):
        assert "backend:" in format_member(_member())

    def test_backend_shows_claude(self):
        result = format_member(_member())
        assert "claude" in result


class TestFormatMemberList:
    def test_table_header_includes_backend(self):
        result = format_member_list(
            [
                _list_entry(
                    agent_id="agent-001",
                    name="Claude-B",
                    coding_agent="claude",
                    pane_id="%7",
                )
            ]
        )
        assert "backend" in result.lower()

    def test_row_shows_claude_backend(self):
        result = format_member_list(
            [
                _list_entry(
                    agent_id="agent-001",
                    name="Claude-B",
                    coding_agent="claude",
                    pane_id="%7",
                )
            ]
        )
        data_lines = [line for line in result.split("\n") if "Claude-B" in line]
        assert len(data_lines) == 1
        assert "claude" in data_lines[0]

    def test_empty_list_unchanged(self):
        assert "0 members" in format_member_list([])


class TestTruncateText:
    def test_none_passes_through(self):
        assert truncate_text(None, full=False) is None

    def test_empty_string_passes_through(self):
        assert truncate_text("", full=False) == ""

    def test_exactly_ten_codepoints_unchanged(self):
        value = "abcdefghij"
        assert len(value) == 10
        assert truncate_text(value, full=False) == "abcdefghij"

    def test_eleven_codepoint_ascii_is_truncated(self):
        value = "abcdefghijk"
        assert len(value) == 11
        assert truncate_text(value, full=False) == "abcdefghij..."

    def test_eleven_codepoint_multibyte_is_truncated_by_codepoint(self):
        value = "あいうえおかきくけこさ"
        assert len(value) == 11
        assert truncate_text(value, full=False) == "あいうえおかきくけこ..."

    def test_full_true_passes_long_string_through(self):
        value = "abcdefghijklmnopqrstuvwxyz"
        assert truncate_text(value, full=True) == value

    def test_full_true_passes_none_through(self):
        assert truncate_text(None, full=True) is None

    def test_custom_limit_is_respected(self):
        assert truncate_text("abcdef", full=False, limit=3) == "abc..."


def _task(text: str | None = "the body of the message") -> dict:
    parts: list[dict] = [{"text": text}] if text is not None else [{}]
    return {
        "id": "task-001",
        "status": {"state": "input_required"},
        "metadata": {
            "fromAgentId": "agent-from",
            "toAgentId": "agent-to",
            "type": "unicast",
        },
        "artifacts": [{"parts": parts}],
    }


class TestTruncateTaskText:
    def test_single_task_shape_truncates_text(self):
        task = _task("abcdefghijklmnop")
        result = truncate_task_text(task, full=False)
        assert result is task
        assert task["artifacts"][0]["parts"][0]["text"] == "abcdefghij..."

    def test_envelope_shape_truncates_text(self):
        envelope = {"task": _task("abcdefghijklmnop")}
        result = truncate_task_text(envelope, full=False)
        assert result is envelope
        assert envelope["task"]["artifacts"][0]["parts"][0]["text"] == "abcdefghij..."

    def test_list_of_tasks_truncates_each(self):
        tasks = [_task("abcdefghijklmnop"), _task("0123456789ABCDEF")]
        result = truncate_task_text(tasks, full=False)
        assert result is tasks
        assert tasks[0]["artifacts"][0]["parts"][0]["text"] == "abcdefghij..."
        assert tasks[1]["artifacts"][0]["parts"][0]["text"] == "0123456789..."

    def test_list_of_envelopes_truncates_each(self):
        items = [{"task": _task("abcdefghijklmnop")}, {"task": _task("short")}]
        truncate_task_text(items, full=False)
        assert items[0]["task"]["artifacts"][0]["parts"][0]["text"] == "abcdefghij..."
        assert items[1]["task"]["artifacts"][0]["parts"][0]["text"] == "short"

    def test_full_true_does_not_mutate(self):
        task = _task("abcdefghijklmnop")
        truncate_task_text(task, full=True)
        assert task["artifacts"][0]["parts"][0]["text"] == "abcdefghijklmnop"

    def test_short_text_is_not_truncated(self):
        task = _task("hello")
        truncate_task_text(task, full=False)
        assert task["artifacts"][0]["parts"][0]["text"] == "hello"

    def test_missing_artifacts_key_is_noop(self):
        task = {
            "id": "task-001",
            "status": {"state": "input_required"},
            "metadata": {"fromAgentId": "a", "type": "unicast"},
        }
        result = truncate_task_text(task, full=False)
        assert result is task
        assert "artifacts" not in task

    def test_missing_parts_key_is_noop(self):
        task = {"artifacts": [{}]}
        truncate_task_text(task, full=False)
        assert task == {"artifacts": [{}]}

    def test_missing_text_key_in_part_is_noop(self):
        task = {"artifacts": [{"parts": [{"data": "binary"}]}]}
        truncate_task_text(task, full=False)
        assert task == {"artifacts": [{"parts": [{"data": "binary"}]}]}
        assert "text" not in task["artifacts"][0]["parts"][0]

    def test_part_with_explicit_none_text_is_left_untouched(self):
        task = {"artifacts": [{"parts": [{"text": None}]}]}
        truncate_task_text(task, full=False)
        assert task["artifacts"][0]["parts"][0]["text"] is None

    def test_mixed_parts_only_truncates_text_bearing(self):
        task = {
            "artifacts": [
                {
                    "parts": [
                        {"text": "abcdefghijklmnop"},
                        {"data": "binary"},
                        {"text": "short"},
                    ]
                }
            ]
        }
        truncate_task_text(task, full=False)
        parts = task["artifacts"][0]["parts"]
        assert parts[0]["text"] == "abcdefghij..."
        assert parts[1] == {"data": "binary"}
        assert parts[2]["text"] == "short"

    def test_multiple_artifacts_each_processed(self):
        task = {
            "artifacts": [
                {"parts": [{"text": "abcdefghijklmnop"}]},
                {"parts": [{"text": "0123456789ABC"}]},
            ]
        }
        truncate_task_text(task, full=False)
        assert task["artifacts"][0]["parts"][0]["text"] == "abcdefghij..."
        assert task["artifacts"][1]["parts"][0]["text"] == "0123456789..."

    def test_non_dict_item_in_list_is_skipped(self):
        items = [None, _task("abcdefghijklmnop")]
        truncate_task_text(items, full=False)
        assert items[0] is None
        assert items[1]["artifacts"][0]["parts"][0]["text"] == "abcdefghij..."

    def test_sibling_metadata_fields_unchanged(self):
        task = _task("abcdefghijklmnop")
        truncate_task_text(task, full=False)
        assert task["id"] == "task-001"
        assert task["status"] == {"state": "input_required"}
        assert task["metadata"] == {
            "fromAgentId": "agent-from",
            "toAgentId": "agent-to",
            "type": "unicast",
        }
