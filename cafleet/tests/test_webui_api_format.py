"""Tests for the ``webui_api`` message formatter.

``_format_messages(rows)`` produces the canonical 11-key message dict
shape consumed by the ``/ui/api`` inbox / sent / timeline endpoints,
batching the agent-name lookup once per call. All three callers feed flat
typed-column task dicts (post-Surface-14) so the formatter inlines the
field mapping rather than parameterising on an accessor.
"""

import pytest

from cafleet import broker, webui_api
from cafleet.tmux import DirectorContext
from cafleet.webui_api import _format_messages


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


_EXPECTED_KEYS = {
    "task_id",
    "from_agent_id",
    "from_agent_name",
    "to_agent_id",
    "to_agent_name",
    "type",
    "status",
    "created_at",
    "status_timestamp",
    "origin_task_id",
    "body",
}


def _create_session() -> dict:
    return broker.create_session(
        director_context=DirectorContext(session="main", window_id="@3", pane_id="%0"),
        coding_agent="claude",
    )


def _two_agents() -> tuple[str, str, str]:
    session = _create_session()
    sid = session["session_id"]
    a = broker.register_agent(session_id=sid, name="alpha", description="A")
    b = broker.register_agent(session_id=sid, name="beta", description="B")
    return sid, a["agent_id"], b["agent_id"]


def _typed_column_row(**overrides) -> dict:
    base = {
        "task_id": "tid-1",
        "context_id": "a2",
        "from_agent_id": "a1",
        "to_agent_id": "a2",
        "type": "unicast",
        "status_state": "input_required",
        "created_at": "2026-04-30T01:00:00+00:00",
        "status_timestamp": "2026-04-30T02:00:00+00:00",
        "origin_task_id": None,
        "text": "hello world",
    }
    base.update(overrides)
    return base


def test_format_messages_empty__empty_rows_returns_empty_and_skips_lookup(monkeypatch):
    calls = []

    def fake_get_agent_names(ids):
        calls.append(list(ids))
        return {}

    monkeypatch.setattr(webui_api.broker, "get_agent_names", fake_get_agent_names)

    result = _format_messages([])
    assert result == []
    assert calls == []


def test_format_messages_batches_lookup__batches_agent_lookup(monkeypatch):
    rows = [
        _typed_column_row(task_id="task-0", from_agent_id="a1", to_agent_id="a2"),
        _typed_column_row(task_id="task-1", from_agent_id="a1", to_agent_id="a3"),
    ]

    get_agent_names_calls = []

    def fake_get_agent_names(ids):
        get_agent_names_calls.append(set(ids))
        return {"a1": "alpha", "a2": "beta", "a3": "gamma"}

    monkeypatch.setattr(webui_api.broker, "get_agent_names", fake_get_agent_names)

    result = _format_messages(rows)

    assert len(get_agent_names_calls) == 1
    assert get_agent_names_calls[0] == {"a1", "a2", "a3"}
    assert len(result) == 2
    assert result[0]["from_agent_name"] == "alpha"
    assert result[0]["to_agent_name"] == "beta"
    assert result[1]["to_agent_name"] == "gamma"


def test_format_messages_shape__output_dict_shape_matches_contract(monkeypatch):
    rows = [_typed_column_row()]

    monkeypatch.setattr(
        webui_api.broker,
        "get_agent_names",
        lambda _ids: {"a1": "alpha", "a2": "beta"},
    )

    result = _format_messages(rows)
    assert len(result) == 1
    assert set(result[0].keys()) == _EXPECTED_KEYS


def test_format_messages_field_mapping__maps_typed_columns_correctly(monkeypatch):
    row = _typed_column_row(
        task_id="tid-2",
        from_agent_id="b1",
        to_agent_id="b2",
        status_state="completed",
        origin_task_id="origin-1",
        text="timeline body",
    )

    monkeypatch.setattr(
        webui_api.broker,
        "get_agent_names",
        lambda _ids: {"b1": "alpha", "b2": "beta"},
    )

    result = _format_messages([row])
    assert len(result) == 1
    msg = result[0]
    assert msg["task_id"] == "tid-2"
    assert msg["from_agent_id"] == "b1"
    assert msg["to_agent_id"] == "b2"
    assert msg["type"] == "unicast"
    assert msg["status"] == "completed"
    assert msg["origin_task_id"] == "origin-1"
    assert msg["body"] == "timeline body"


def test_format_messages_end_to_end_inbox__inbox_rows_through_format_messages_match_contract():
    sid, sender, recipient = _two_agents()
    broker.send_message(sid, sender, recipient, "snapshot body")

    rows = broker.list_inbox(recipient)
    result = _format_messages(rows)

    assert len(result) == 1
    msg = result[0]
    assert set(msg.keys()) == _EXPECTED_KEYS
    assert msg["from_agent_id"] == sender
    assert msg["from_agent_name"] == "alpha"
    assert msg["to_agent_id"] == recipient
    assert msg["to_agent_name"] == "beta"
    assert msg["type"] == "unicast"
    assert msg["status"] == "input_required"
    assert msg["body"] == "snapshot body"
    assert msg["origin_task_id"] is None
    assert isinstance(msg["task_id"], str)
    assert msg["task_id"]
    assert isinstance(msg["created_at"], str)
    assert msg["created_at"]
    assert isinstance(msg["status_timestamp"], str)
    assert msg["status_timestamp"]


def test_format_messages_end_to_end_timeline__timeline_entries_through_format_messages_match_contract():
    sid, sender, recipient = _two_agents()
    broker.send_message(sid, sender, recipient, "timeline snapshot")

    rows = broker.list_timeline(sid)
    result = _format_messages(rows)

    assert len(result) == 1
    msg = result[0]
    assert set(msg.keys()) == _EXPECTED_KEYS
    assert msg["from_agent_id"] == sender
    assert msg["from_agent_name"] == "alpha"
    assert msg["to_agent_id"] == recipient
    assert msg["to_agent_name"] == "beta"
    assert msg["type"] == "unicast"
    assert msg["status"] == "input_required"
    assert msg["body"] == "timeline snapshot"
    assert isinstance(msg["task_id"], str)
    assert msg["task_id"]
    assert isinstance(msg["created_at"], str)
    assert msg["created_at"]
    assert isinstance(msg["status_timestamp"], str)
    assert msg["status_timestamp"]
