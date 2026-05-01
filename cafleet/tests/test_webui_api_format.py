"""Tests for the ``webui_api`` message formatter.

``_format_messages(rows, accessor)`` produces the canonical 11-key
message dict shape consumed by the ``/ui/api`` inbox / sent / timeline
endpoints, batching the agent-name lookup once per call.
``_raw_task_accessor`` adapts ``broker.list_inbox`` / ``list_sent`` rows
and ``_timeline_entry_accessor`` adapts ``broker.list_timeline`` entries
to the merger's expected per-row shape.
"""

import pytest
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

import cafleet.db.engine  # noqa: F401 — registers PRAGMA listener globally
from cafleet import broker, webui_api
from cafleet.db.models import Base
from cafleet.tmux import DirectorContext
from cafleet.webui_api import (
    _format_messages,
    _raw_task_accessor,
    _timeline_entry_accessor,
)

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


@pytest.fixture
def sync_sessionmaker():
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    return sessionmaker(engine, expire_on_commit=False)


@pytest.fixture
def _patch_broker(sync_sessionmaker, monkeypatch):
    monkeypatch.setattr(broker, "get_sync_sessionmaker", lambda: sync_sessionmaker)


@pytest.fixture(autouse=True)
def broker_session(sync_sessionmaker, _patch_broker):
    return sync_sessionmaker


def _create_session() -> dict:
    return broker.create_session(
        director_context=DirectorContext(session="main", window_id="@3", pane_id="%0"),
    )


def _two_agents() -> tuple[str, str, str]:
    session = _create_session()
    sid = session["session_id"]
    a = broker.register_agent(session_id=sid, name="alpha", description="A")
    b = broker.register_agent(session_id=sid, name="beta", description="B")
    return sid, a["agent_id"], b["agent_id"]


def test_format_messages_empty__empty_rows_returns_empty_and_skips_lookup(monkeypatch):
    calls = []

    def fake_get_agent_names(ids):
        calls.append(list(ids))
        return {}

    monkeypatch.setattr(webui_api.broker, "get_agent_names", fake_get_agent_names)

    accessor_calls = []

    def accessor(row):
        accessor_calls.append(row)
        return {}

    result = _format_messages([], accessor)
    assert result == []
    assert calls == []
    assert accessor_calls == []


def test_format_messages_batches_lookup__calls_accessor_per_row_and_batches_agent_lookup(
    monkeypatch,
):
    rows = [
        {"row_index": 0, "from": "a1", "to": "a2"},
        {"row_index": 1, "from": "a1", "to": "a3"},
    ]

    accessor_inputs = []

    def accessor(row):
        accessor_inputs.append(row)
        return {
            "task_id": f"task-{row['row_index']}",
            "from_id": row["from"],
            "to_id": row["to"],
            "type_": "message",
            "status": "input_required",
            "created_at": "2026-04-30T00:00:00+00:00",
            "status_timestamp": "2026-04-30T00:00:00+00:00",
            "origin_task_id": None,
            "body": f"body-{row['row_index']}",
        }

    get_agent_names_calls = []

    def fake_get_agent_names(ids):
        get_agent_names_calls.append(set(ids))
        return {"a1": "alpha", "a2": "beta", "a3": "gamma"}

    monkeypatch.setattr(webui_api.broker, "get_agent_names", fake_get_agent_names)

    result = _format_messages(rows, accessor)

    assert accessor_inputs == rows
    assert len(get_agent_names_calls) == 1
    assert get_agent_names_calls[0] == {"a1", "a2", "a3"}
    assert len(result) == 2
    assert result[0]["from_agent_name"] == "alpha"
    assert result[0]["to_agent_name"] == "beta"
    assert result[1]["to_agent_name"] == "gamma"


def test_format_messages_shape__output_dict_shape_matches_contract(monkeypatch):
    rows = [{"x": 1}]

    def accessor(row):
        return {
            "task_id": "t1",
            "from_id": "a1",
            "to_id": "a2",
            "type_": "message",
            "status": "input_required",
            "created_at": "2026-04-30T00:00:00+00:00",
            "status_timestamp": "2026-04-30T00:00:00+00:00",
            "origin_task_id": None,
            "body": "hello",
        }

    monkeypatch.setattr(
        webui_api.broker,
        "get_agent_names",
        lambda _ids: {"a1": "alpha", "a2": "beta"},
    )

    result = _format_messages(rows, accessor)
    assert len(result) == 1
    assert set(result[0].keys()) == _EXPECTED_KEYS


def test_raw_task_accessor__extracts_from_broker_inbox_row_shape():
    row = {
        "task_id": "tid-1",
        "from_agent_id": "a1",
        "to_agent_id": "a2",
        "type": "unicast",
        "status_state": "input_required",
        "created_at": "2026-04-30T01:00:00+00:00",
        "status_timestamp": "2026-04-30T02:00:00+00:00",
        "origin_task_id": None,
        "task_json": (
            '{"id":"tid-1","artifacts":[{"parts":[{"text":"hello world"}]}]}'
        ),
    }

    result = _raw_task_accessor(row)

    assert result["task_id"] == "tid-1"
    assert result["from_id"] == "a1"
    assert result["to_id"] == "a2"
    assert result["type_"] == "unicast"
    assert result["status"] == "input_required"
    assert result["created_at"] == "2026-04-30T01:00:00+00:00"
    assert result["status_timestamp"] == "2026-04-30T02:00:00+00:00"
    assert result["origin_task_id"] is None
    assert result["body"] == "hello world"


def test_timeline_entry_accessor__extracts_from_broker_list_timeline_entry_shape():
    entry = {
        "task": {
            "id": "tid-2",
            "status": {
                "state": "completed",
                "timestamp": "2026-04-30T03:00:00+00:00",
            },
            "metadata": {
                "fromAgentId": "b1",
                "toAgentId": "b2",
                "type": "unicast",
            },
            "artifacts": [{"parts": [{"text": "timeline body"}]}],
        },
        "created_at": "2026-04-30T03:30:00+00:00",
        "origin_task_id": "origin-1",
    }

    result = _timeline_entry_accessor(entry)

    assert result["task_id"] == "tid-2"
    assert result["from_id"] == "b1"
    assert result["to_id"] == "b2"
    assert result["type_"] == "unicast"
    assert result["status"] == "completed"
    assert result["created_at"] == "2026-04-30T03:30:00+00:00"
    assert result["status_timestamp"] == "2026-04-30T03:00:00+00:00"
    assert result["origin_task_id"] == "origin-1"
    assert result["body"] == "timeline body"


def test_format_messages_end_to_end_raw_task_accessor__inbox_rows_through_format_messages_match_contract():
    sid, sender, recipient = _two_agents()
    broker.send_message(sid, sender, recipient, "snapshot body")

    rows = broker.list_inbox(recipient)
    result = _format_messages(rows, _raw_task_accessor)

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


def test_format_messages_end_to_end_timeline_entry_accessor__timeline_entries_through_format_messages_match_contract():
    sid, sender, recipient = _two_agents()
    broker.send_message(sid, sender, recipient, "timeline snapshot")

    entries = broker.list_timeline(sid)
    result = _format_messages(entries, _timeline_entry_accessor)

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
