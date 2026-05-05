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


def test_format_member__includes_backend_field():
    """Compact 1-line render uses ``backend=<name>`` (post-Surface-3)."""
    assert "backend=" in format_member(_member())


def test_format_member__backend_shows_claude():
    result = format_member(_member())
    assert "claude" in result


def test_format_member_list__table_header_includes_backend():
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


def test_format_member_list__row_shows_claude_backend():
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


def test_format_member_list__empty_list_unchanged():
    assert "0 members" in format_member_list([])


def test_truncate_text__none_passes_through():
    assert truncate_text(None, full=False, limit=10) is None


def test_truncate_text__empty_string_passes_through():
    assert truncate_text("", full=False, limit=10) == ""


def test_truncate_text__exactly_ten_codepoints_unchanged():
    value = "abcdefghij"
    assert len(value) == 10
    assert truncate_text(value, full=False, limit=10) == "abcdefghij"


def test_truncate_text__eleven_codepoint_ascii_is_truncated():
    value = "abcdefghijk"
    assert len(value) == 11
    assert truncate_text(value, full=False, limit=10) == "abcdefghij…"


def test_truncate_text__eleven_codepoint_multibyte_is_truncated_by_codepoint():
    value = "あいうえおかきくけこさ"
    assert len(value) == 11
    assert truncate_text(value, full=False, limit=10) == "あいうえおかきくけこ…"


def test_truncate_text__full_true_passes_long_string_through():
    value = "abcdefghijklmnopqrstuvwxyz"
    assert truncate_text(value, full=True) == value


def test_truncate_text__full_true_passes_none_through():
    assert truncate_text(None, full=True) is None


def test_truncate_text__custom_limit_is_respected():
    assert truncate_text("abcdef", full=False, limit=3) == "abc…"


def _task(text: str | None = "the body of the message") -> dict:
    """Build a flat typed-column task dict (post-Surface-14 shape)."""
    base: dict = {
        "task_id": "task-001",
        "context_id": "agent-to",
        "from_agent_id": "agent-from",
        "to_agent_id": "agent-to",
        "type": "unicast",
        "created_at": "2026-05-05T12:00:00.000000+00:00",
        "status_state": "input_required",
        "status_timestamp": "2026-05-05T12:00:00.000000+00:00",
        "origin_task_id": None,
    }
    if text is not None:
        base["text"] = text
    return base


def test_truncate_task_text__single_task_shape_truncates_text():
    task = _task("abcdefghijklmnop")
    result = truncate_task_text(task, full=False, limit=10)
    assert result is task
    assert task["text"] == "abcdefghij…"


def test_truncate_task_text__envelope_shape_truncates_text():
    envelope = {"task": _task("abcdefghijklmnop")}
    result = truncate_task_text(envelope, full=False, limit=10)
    assert result is envelope
    assert envelope["task"]["text"] == "abcdefghij…"


def test_truncate_task_text__list_of_tasks_truncates_each():
    tasks = [_task("abcdefghijklmnop"), _task("0123456789ABCDEF")]
    result = truncate_task_text(tasks, full=False, limit=10)
    assert result is tasks
    assert tasks[0]["text"] == "abcdefghij…"
    assert tasks[1]["text"] == "0123456789…"


def test_truncate_task_text__list_of_envelopes_truncates_each():
    items = [{"task": _task("abcdefghijklmnop")}, {"task": _task("short")}]
    truncate_task_text(items, full=False, limit=10)
    assert items[0]["task"]["text"] == "abcdefghij…"
    assert items[1]["task"]["text"] == "short"


def test_truncate_task_text__full_true_does_not_mutate():
    task = _task("abcdefghijklmnop")
    truncate_task_text(task, full=True)
    assert task["text"] == "abcdefghijklmnop"


def test_truncate_task_text__short_text_is_not_truncated():
    task = _task("hello")
    truncate_task_text(task, full=False, limit=10)
    assert task["text"] == "hello"


def test_truncate_task_text__missing_text_key_is_noop():
    """Tasks without a 'text' key (e.g. legacy partial fixtures) pass through unchanged."""
    task: dict = {
        "task_id": "task-001",
        "context_id": "ctx",
        "status_state": "input_required",
    }
    result = truncate_task_text(task, full=False, limit=10)
    assert result is task
    assert "text" not in task


def test_truncate_task_text__non_dict_item_in_list_is_skipped():
    items = [None, _task("abcdefghijklmnop")]
    truncate_task_text(items, full=False, limit=10)
    assert items[0] is None
    assert items[1]["text"] == "abcdefghij…"


def test_truncate_task_text__sibling_typed_column_fields_unchanged():
    task = _task("abcdefghijklmnop")
    truncate_task_text(task, full=False, limit=10)
    assert task["task_id"] == "task-001"
    assert task["status_state"] == "input_required"
    assert task["from_agent_id"] == "agent-from"
    assert task["to_agent_id"] == "agent-to"
    assert task["type"] == "unicast"
