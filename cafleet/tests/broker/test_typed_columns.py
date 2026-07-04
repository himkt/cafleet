"""Typed-column broker shape.

Every public broker caller returns a flat dict whose keys map 1:1 onto the
typed ``Task`` columns plus the ``text`` body column — no ``metadata``
wrapping, no ``artifacts``/``parts``/``kind``/``history``, no leftover
``task_json`` field on result rows.
"""

import pytest
from sqlalchemy import create_engine, text

from cafleet import broker
from cafleet.broker import _shared, messaging
from cafleet.db.models import Base, Task
from tests.broker._helpers import (
    _create_fleet,
    _register_agent,
    _setup_three_agents,
    _setup_two_agents,
)

REQUIRED_TASK_KEYS = {
    "task_id",
    "context_id",
    "from_agent_id",
    "to_agent_id",
    "type",
    "created_at",
    "status_state",
    "status_timestamp",
    "origin_task_id",
    "text",
}


def _assert_flat_typed_shape(d: dict, *, expect_type: str | None = None) -> None:
    assert set(d.keys()) == REQUIRED_TASK_KEYS, (
        f"unexpected typed-column key set: {sorted(d.keys())}"
    )
    if expect_type is not None:
        assert d["type"] == expect_type


def test_send_message__unicast_returns_flat_typed_envelope():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Did the API change?")
    task = result["task"]

    assert set(task.keys()) == REQUIRED_TASK_KEYS
    assert task["type"] == "unicast"
    assert task["from_agent_id"] == sender
    assert task["to_agent_id"] == recipient
    assert task["context_id"] == recipient
    assert task["text"] == "Did the API change?"
    assert task["status_state"] == "input_required"
    assert task["origin_task_id"] is None
    assert isinstance(task["task_id"], int)
    assert "notification_sent" in result


@pytest.mark.parametrize(
    ("origin_in", "origin_out"),
    [(None, None), (777, 777)],
)
def test_send_message__origin_task_id_default_and_propagate(origin_in, origin_out):
    now = "2026-05-05T12:00:00.000000+00:00"
    kwargs = {"origin_task_id": origin_in} if origin_in is not None else {}
    result = messaging._unicast_task_dict(
        recipient_id=2,
        sender_id=1,
        text="Hello",
        now=now,
        **kwargs,
    )
    assert result["origin_task_id"] == origin_out


def test_broadcast_message__summary_envelope_shape():
    sid, sender, _b, _c = _setup_three_agents()
    [result] = broker.broadcast_message(sid, sender, "Attention all")
    summary = result["task"]

    assert set(summary.keys()) == REQUIRED_TASK_KEYS
    assert summary["type"] == "broadcast_summary"
    assert summary["from_agent_id"] == sender
    assert summary["context_id"] == sender
    assert summary["to_agent_id"] is None
    assert summary["status_state"] == "completed"
    assert "Broadcast sent" in summary["text"]
    assert isinstance(result["recipients"], int)
    assert isinstance(result["delivered"], int)


def test_broadcast_message__delivery_task_shape_and_origin_link():
    sid, sender, b_id, _c = _setup_three_agents()
    [result] = broker.broadcast_message(sid, sender, "delivery body")
    summary_id = result["task"]["task_id"]

    [delivered] = broker.poll_tasks(b_id)
    _assert_flat_typed_shape(delivered, expect_type="unicast")
    assert delivered["text"] == "delivery body"
    assert delivered["origin_task_id"] == summary_id


def test_poll_tasks__returns_flat_typed_task_dicts():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")

    rows = broker.poll_tasks(recipient)
    assert len(rows) == 2
    for row in rows:
        _assert_flat_typed_shape(row, expect_type="unicast")
    assert {r["text"] for r in rows} == {"first", "second"}

    sid2, sender2, _b, _c = _setup_three_agents()
    broker.broadcast_message(sid2, sender2, "broadcast body")
    sender_tasks = broker.poll_tasks(sender2)
    assert "broadcast_summary" not in [t["type"] for t in sender_tasks]


@pytest.mark.parametrize(
    ("action", "expected_state"),
    [("ack", "completed"), ("cancel", "canceled")],
)
def test_ack_and_cancel__transition_and_round_trip(action, expected_state):
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, f"round trip {action}")
    tid = sent["task"]["task_id"]

    if action == "ack":
        result = broker.ack_task(recipient, tid)
        unauthorized_actor = sender
        unauthorized_call = broker.ack_task
    else:
        result = broker.cancel_task(sender, tid)
        unauthorized_actor = recipient
        unauthorized_call = broker.cancel_task

    _assert_flat_typed_shape(result["task"], expect_type="unicast")
    assert result["task"]["status_state"] == expected_state

    # poll now returns only un-acked deliveries; verify persistence via get_task.
    persisted = broker.get_task(sid, tid)["task"]
    assert persisted["text"] == f"round trip {action}"
    assert persisted["status_state"] == expected_state

    sent2 = broker.send_message(sid, sender, recipient, "unauthorized check")
    with pytest.raises(PermissionError):
        unauthorized_call(unauthorized_actor, sent2["task"]["task_id"])


@pytest.mark.parametrize("api", ["list_inbox", "list_sent", "list_timeline"])
def test_list_apis__no_metadata_wrapping_and_filter_broadcast_summary(api):
    sid, sender, _b, _c = _setup_three_agents()
    broker.send_message(sid, sender, _b, "direct body")
    broker.broadcast_message(sid, sender, "broadcast body")

    if api == "list_inbox":
        rows = broker.list_inbox(_b)
        tasks = rows
    elif api == "list_sent":
        rows = broker.list_sent(sender)
        tasks = rows
    else:
        rows = broker.list_timeline(sid)
        tasks = [r.get("task") if isinstance(r.get("task"), dict) else r for r in rows]

    assert len(tasks) >= 1
    for task in tasks:
        for forbidden in ("metadata", "artifacts", "history", "task_json"):
            assert forbidden not in task

    if api == "list_sent":
        assert "broadcast_summary" not in [t["type"] for t in tasks]
    if api == "list_timeline":
        assert "broadcast_summary" not in [t.get("type") for t in tasks if "type" in t]


def test_task_table_and_model__text_column_present_task_json_absent():
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    try:
        with engine.connect() as conn:
            rows = conn.execute(text("PRAGMA table_info(tasks)")).fetchall()
        columns = {r[1] for r in rows}
    finally:
        engine.dispose()
    assert "text" in columns
    assert "task_json" not in columns

    assert hasattr(Task, "text")
    assert not hasattr(Task, "task_json")

    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "via send_message body")
    sm = _shared.get_sync_sessionmaker()
    with sm() as s:
        row = s.execute(
            text("SELECT text FROM tasks WHERE task_id = :tid"),
            {"tid": sent["task"]["task_id"]},
        ).fetchone()
    assert row is not None
    assert row[0] == "via send_message body"

    # Cross-fleet boundary check (subsumes test_get_task__rejects_task_not_in_fleet
    # & test_read_task__returns_none_for_missing_task)
    other = _create_fleet()
    other_sender = _register_agent(other["fleet_id"], name="outsider-sender")
    other_recipient = _register_agent(other["fleet_id"], name="outsider-recipient")
    foreign_sent = broker.send_message(
        other["fleet_id"],
        other_sender["agent_id"],
        other_recipient["agent_id"],
        "elsewhere",
    )
    with pytest.raises(ValueError, match="not found"):
        broker.get_task(sid, foreign_sent["task"]["task_id"])
