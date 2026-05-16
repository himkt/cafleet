"""Tests for ``cafleet.output`` formatting helpers."""

import pytest

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


def _list_entry(*, agent_id, name, coding_agent, pane_id):
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


def _task(text="the body of the message") -> dict:
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


def test_format_member__compact_includes_backend_and_value():
    result = format_member(_member())
    assert "backend=" in result
    assert "claude" in result


def test_format_member_list__header_and_row_shape_and_empty_message():
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
    data_lines = [line for line in result.split("\n") if "Claude-B" in line]
    assert len(data_lines) == 1
    assert "claude" in data_lines[0]
    # Empty list returns the headline message.
    assert "0 members" in format_member_list([])


@pytest.mark.parametrize(
    ("scenario", "value", "kwargs", "expected"),
    [
        ("none_passthrough", None, {"full": False, "limit": 10}, None),
        ("empty_passthrough", "", {"full": False, "limit": 10}, ""),
        (
            "exact_limit_unchanged",
            "abcdefghij",
            {"full": False, "limit": 10},
            "abcdefghij",
        ),
        (
            "over_limit_truncated",
            "abcdefghijk",
            {"full": False, "limit": 10},
            "abcdefghij…",
        ),
        (
            "multibyte_truncated_by_codepoint",
            "あいうえおかきくけこさ",
            {"full": False, "limit": 10},
            "あいうえおかきくけこ…",
        ),
        (
            "full_passthrough_long",
            "abcdefghijklmnopqrstuvwxyz",
            {"full": True},
            "abcdefghijklmnopqrstuvwxyz",
        ),
        ("full_passthrough_none", None, {"full": True}, None),
        ("custom_limit_three", "abcdef", {"full": False, "limit": 3}, "abc…"),
    ],
)
def test_truncate_text__matrix(scenario, value, kwargs, expected):
    assert truncate_text(value, **kwargs) == expected


@pytest.mark.parametrize(
    ("scenario", "make_input", "expected_text"),
    [
        ("single_task", lambda: _task("abcdefghijklmnop"), "abcdefghij…"),
        ("envelope_shape", lambda: {"task": _task("abcdefghijklmnop")}, "abcdefghij…"),
        ("short_text_unchanged", lambda: _task("hello"), "hello"),
    ],
)
def test_truncate_task_text__single_envelope_shapes(
    scenario, make_input, expected_text
):
    data = make_input()
    result = truncate_task_text(data, full=False, limit=10)
    assert result is data
    task = data.get("task", data)
    assert task["text"] == expected_text


def test_truncate_task_text__list_shapes_and_non_dict_skipped():
    tasks = [_task("abcdefghijklmnop"), _task("0123456789ABCDEF")]
    truncate_task_text(tasks, full=False, limit=10)
    assert tasks[0]["text"] == "abcdefghij…"
    assert tasks[1]["text"] == "0123456789…"

    envelopes = [{"task": _task("abcdefghijklmnop")}, {"task": _task("short")}]
    truncate_task_text(envelopes, full=False, limit=10)
    assert envelopes[0]["task"]["text"] == "abcdefghij…"
    assert envelopes[1]["task"]["text"] == "short"

    # Non-dict items in a list are skipped silently.
    mixed: list = [None, _task("abcdefghijklmnop")]
    truncate_task_text(mixed, full=False, limit=10)
    assert mixed[0] is None
    assert mixed[1]["text"] == "abcdefghij…"


def test_truncate_task_text__full_true_does_not_mutate():
    task = _task("abcdefghijklmnop")
    truncate_task_text(task, full=True)
    assert task["text"] == "abcdefghijklmnop"


def test_truncate_task_text__missing_text_key_is_noop_and_siblings_unchanged():
    no_text_task: dict = {
        "task_id": "task-001",
        "context_id": "ctx",
        "status_state": "input_required",
    }
    result = truncate_task_text(no_text_task, full=False, limit=10)
    assert result is no_text_task
    assert "text" not in no_text_task

    task = _task("abcdefghijklmnop")
    truncate_task_text(task, full=False, limit=10)
    assert task["task_id"] == "task-001"
    assert task["status_state"] == "input_required"
    assert task["from_agent_id"] == "agent-from"
    assert task["to_agent_id"] == "agent-to"
    assert task["type"] == "unicast"
