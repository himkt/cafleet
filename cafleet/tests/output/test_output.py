"""Tests for ``cafleet.output`` formatting helpers."""

import pytest

from cafleet.output import (
    format_member,
    format_member_list,
    truncate_message_text,
    truncate_text,
)


def _member(**placement_overrides) -> dict:
    placement = {
        "mux_pane_id": "%7",
        "mux_window_id": "@3",
        "coding_agent": "claude",
    }
    placement.update(placement_overrides)
    return {
        "member_id": 1,
        "name": "Claude-B",
        "placement": placement,
    }


def _list_entry(*, member_id, name, coding_agent, pane_id):
    return {
        "member_id": member_id,
        "name": name,
        "kind": "member",
        "placement": {
            "mux_session": "main",
            "mux_window_id": "@3",
            "mux_pane_id": pane_id,
            "coding_agent": coding_agent,
            "created_at": "2026-04-12T10:15:00Z",
        },
        "last_sent": None,
        "last_recv": None,
        "last_ack": None,
        "idle": None,
    }


def _message(text="the body of the message") -> dict:
    base: dict = {
        "message_id": 1,
        "owner_member_id": 20,
        "from_member_id": 10,
        "to_member_id": 20,
        "type": "unicast",
        "created_at": "2026-05-05T12:00:00.000000+00:00",
        "status_state": "input_required",
        "status_timestamp": "2026-05-05T12:00:00.000000+00:00",
        "origin_message_id": None,
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
                member_id=1,
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
        ("single_message", lambda: _message("abcdefghijklmnop"), "abcdefghij…"),
        (
            "envelope_shape",
            lambda: {"message": _message("abcdefghijklmnop")},
            "abcdefghij…",
        ),
        ("short_text_unchanged", lambda: _message("hello"), "hello"),
    ],
)
def test_truncate_message_text__single_envelope_shapes(
    monkeypatch, scenario, make_input, expected_text
):
    monkeypatch.setattr("cafleet.config.settings.max_text_len", 10)
    data = make_input()
    result = truncate_message_text(data, full=False)
    assert result is data
    message = data.get("message", data)
    assert message["text"] == expected_text


def test_truncate_message_text__list_shapes_and_non_dict_skipped(monkeypatch):
    monkeypatch.setattr("cafleet.config.settings.max_text_len", 10)
    messages = [_message("abcdefghijklmnop"), _message("0123456789ABCDEF")]
    truncate_message_text(messages, full=False)
    assert messages[0]["text"] == "abcdefghij…"
    assert messages[1]["text"] == "0123456789…"

    envelopes = [
        {"message": _message("abcdefghijklmnop")},
        {"message": _message("short")},
    ]
    truncate_message_text(envelopes, full=False)
    assert envelopes[0]["message"]["text"] == "abcdefghij…"
    assert envelopes[1]["message"]["text"] == "short"

    # Non-dict items in a list are skipped silently.
    mixed: list = [None, _message("abcdefghijklmnop")]
    truncate_message_text(mixed, full=False)
    assert mixed[0] is None
    assert mixed[1]["text"] == "abcdefghij…"


def test_truncate_message_text__full_true_does_not_mutate():
    message = _message("abcdefghijklmnop")
    truncate_message_text(message, full=True)
    assert message["text"] == "abcdefghijklmnop"


def test_truncate_message_text__missing_text_key_is_noop_and_siblings_unchanged(
    monkeypatch,
):
    monkeypatch.setattr("cafleet.config.settings.max_text_len", 10)
    no_text_message: dict = {
        "message_id": 1,
        "owner_member_id": 20,
        "status_state": "input_required",
    }
    result = truncate_message_text(no_text_message, full=False)
    assert result is no_text_message
    assert "text" not in no_text_message

    message = _message("abcdefghijklmnop")
    truncate_message_text(message, full=False)
    assert message["message_id"] == 1
    assert message["status_state"] == "input_required"
    assert message["from_member_id"] == 10
    assert message["to_member_id"] == 20
    assert message["type"] == "unicast"
