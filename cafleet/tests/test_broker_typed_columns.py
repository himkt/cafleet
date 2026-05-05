"""Post-Surface-14 typed-column broker shape (design 0000049 Step 2).

Every public and private broker caller that previously round-tripped through
``Task.task_json`` must, after Surface 14, return a flat dict whose keys map
1:1 onto the typed ``Task`` columns plus the new ``text`` body column. No
``metadata`` wrapping, no ``artifacts``/``parts``/``kind``/``history``, and
no leftover ``task_json`` field on result rows.
"""

import uuid

import pytest
from sqlalchemy import create_engine, text
from sqlalchemy.orm import sessionmaker

import cafleet.db.engine  # noqa: F401 — registers PRAGMA listener globally
from cafleet import broker
from cafleet.db.models import Base, Task
from cafleet.tmux import DirectorContext


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


def _create_session(label: str | None = None) -> dict:
    return broker.create_session(
        label=label,
        director_context=DirectorContext(session="main", window_id="@3", pane_id="%0"),
        coding_agent="claude",
    )


def _register_agent(
    session_id: str,
    name: str = "test-agent",
    description: str = "A test agent",
) -> dict:
    return broker.register_agent(
        session_id=session_id,
        name=name,
        description=description,
    )


def _setup_two_agents() -> tuple[str, str, str]:
    s = _create_session()
    sid = s["session_id"]
    sender = _register_agent(sid, name="sender")
    recipient = _register_agent(sid, name="recipient")
    return sid, sender["agent_id"], recipient["agent_id"]


def _setup_three_agents() -> tuple[str, str, str, str]:
    s = _create_session()
    sid = s["session_id"]
    a = _register_agent(sid, name="agent-a")
    b = _register_agent(sid, name="agent-b")
    c = _register_agent(sid, name="agent-c")
    return sid, a["agent_id"], b["agent_id"], c["agent_id"]


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

# Old-shape camelCase keys plus the legacy nesting wrappers that Surface 14
# eliminates. ``task_json`` is also forbidden — it was the column name and
# any leftover render-side key by the same name signals an incomplete cleanup.
FORBIDDEN_LEGACY_KEYS = {
    "id",
    "contextId",
    "fromAgentId",
    "toAgentId",
    "originTaskId",
    "metadata",
    "artifacts",
    "history",
    "kind",
    "parts",
    "task_json",
}


def _assert_flat_typed_shape(d: dict, *, expect_type: str | None = None) -> None:
    missing = REQUIRED_TASK_KEYS - d.keys()
    assert not missing, f"missing required typed-column keys: {sorted(missing)}"
    forbidden = FORBIDDEN_LEGACY_KEYS & d.keys()
    assert not forbidden, f"forbidden legacy keys still present: {sorted(forbidden)}"
    if expect_type is not None:
        assert d["type"] == expect_type


# ---------------------------------------------------------------------------
# 1. broker._unicast_task_dict — flat dict with typed-column keys
# ---------------------------------------------------------------------------


def test_unicast_task_dict__returns_flat_typed_shape():
    now = "2026-05-05T12:00:00.000000+00:00"
    result = broker._unicast_task_dict(
        recipient_id="rid",
        sender_id="sid",
        text="Hello",
        now=now,
    )
    _assert_flat_typed_shape(result, expect_type="unicast")


def test_unicast_task_dict__no_metadata_wrapping():
    now = "2026-05-05T12:00:00.000000+00:00"
    result = broker._unicast_task_dict(
        recipient_id="rid",
        sender_id="sid",
        text="Hello",
        now=now,
    )
    assert "metadata" not in result
    assert "artifacts" not in result
    assert "history" not in result


def test_unicast_task_dict__text_field_holds_body():
    now = "2026-05-05T12:00:00.000000+00:00"
    result = broker._unicast_task_dict(
        recipient_id="rid",
        sender_id="sid",
        text="Did the API change?",
        now=now,
    )
    assert result["text"] == "Did the API change?"


def test_unicast_task_dict__from_agent_id_top_level():
    now = "2026-05-05T12:00:00.000000+00:00"
    result = broker._unicast_task_dict(
        recipient_id="rid",
        sender_id="sid",
        text="Hello",
        now=now,
    )
    assert result["from_agent_id"] == "sid"


def test_unicast_task_dict__to_agent_id_top_level():
    now = "2026-05-05T12:00:00.000000+00:00"
    result = broker._unicast_task_dict(
        recipient_id="rid",
        sender_id="sid",
        text="Hello",
        now=now,
    )
    assert result["to_agent_id"] == "rid"


def test_unicast_task_dict__context_id_is_recipient():
    now = "2026-05-05T12:00:00.000000+00:00"
    result = broker._unicast_task_dict(
        recipient_id="rid",
        sender_id="sid",
        text="Hello",
        now=now,
    )
    assert result["context_id"] == "rid"


def test_unicast_task_dict__type_is_unicast():
    now = "2026-05-05T12:00:00.000000+00:00"
    result = broker._unicast_task_dict(
        recipient_id="rid",
        sender_id="sid",
        text="Hello",
        now=now,
    )
    assert result["type"] == "unicast"


def test_unicast_task_dict__task_id_is_valid_uuid():
    now = "2026-05-05T12:00:00.000000+00:00"
    result = broker._unicast_task_dict(
        recipient_id="rid",
        sender_id="sid",
        text="Hello",
        now=now,
    )
    uuid.UUID(result["task_id"])


def test_unicast_task_dict__status_state_is_input_required():
    now = "2026-05-05T12:00:00.000000+00:00"
    result = broker._unicast_task_dict(
        recipient_id="rid",
        sender_id="sid",
        text="Hello",
        now=now,
    )
    assert result["status_state"] == "input_required"


def test_unicast_task_dict__status_timestamp_matches_now_arg():
    now = "2026-05-05T12:00:00.000000+00:00"
    result = broker._unicast_task_dict(
        recipient_id="rid",
        sender_id="sid",
        text="Hello",
        now=now,
    )
    assert result["status_timestamp"] == now


def test_unicast_task_dict__origin_task_id_default_none():
    now = "2026-05-05T12:00:00.000000+00:00"
    result = broker._unicast_task_dict(
        recipient_id="rid",
        sender_id="sid",
        text="Hello",
        now=now,
    )
    assert result["origin_task_id"] is None


def test_unicast_task_dict__origin_task_id_propagates_when_supplied():
    now = "2026-05-05T12:00:00.000000+00:00"
    origin = str(uuid.uuid4())
    result = broker._unicast_task_dict(
        recipient_id="rid",
        sender_id="sid",
        text="Hello",
        now=now,
        origin_task_id=origin,
    )
    assert result["origin_task_id"] == origin


# ---------------------------------------------------------------------------
# 2. send_message — flat typed-column return
# ---------------------------------------------------------------------------


def test_send_message__task_is_flat_typed_shape():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Hello")
    _assert_flat_typed_shape(result["task"], expect_type="unicast")


def test_send_message__task_text_holds_body():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Did the API change?")
    assert result["task"]["text"] == "Did the API change?"


def test_send_message__no_metadata_or_artifacts_wrapping():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Hello")
    task = result["task"]
    assert "metadata" not in task
    assert "artifacts" not in task
    assert "history" not in task


def test_send_message__top_level_agent_ids():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Hello")
    task = result["task"]
    assert task["from_agent_id"] == sender
    assert task["to_agent_id"] == recipient
    assert task["context_id"] == recipient


def test_send_message__notification_sent_at_wrapper_level():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Hello")
    assert "notification_sent" in result


# ---------------------------------------------------------------------------
# 3. _save_task + schema — typed columns, no task_json column
# ---------------------------------------------------------------------------


def test_task_table__has_text_column_no_task_json_column():
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    try:
        with engine.connect() as conn:
            rows = conn.execute(text("PRAGMA table_info(tasks)")).fetchall()
        columns = {r[1] for r in rows}
    finally:
        engine.dispose()
    assert "text" in columns, f"text column missing; got {sorted(columns)}"
    assert "task_json" not in columns, (
        f"task_json column should be dropped; got {sorted(columns)}"
    )


def test_task_model__has_text_attribute_no_task_json_attribute():
    assert hasattr(Task, "text"), "Task.text mapped attribute is missing"
    assert not hasattr(Task, "task_json"), (
        "Task.task_json should be removed from the ORM model"
    )


def test_save_task__writes_text_column_directly():
    sid, sender, recipient = _setup_two_agents()
    task_dict = broker._unicast_task_dict(
        recipient_id=recipient,
        sender_id=sender,
        text="direct save body",
        now="2026-05-05T12:00:00.000000+00:00",
    )
    sm = broker.get_sync_sessionmaker()
    with sm() as s, s.begin():
        broker._save_task(s, task_dict)

    sm2 = broker.get_sync_sessionmaker()
    with sm2() as s:
        row = s.execute(
            text("SELECT text FROM tasks WHERE task_id = :tid"),
            {"tid": task_dict["task_id"]},
        ).fetchone()
    assert row is not None
    assert row[0] == "direct save body"


def test_save_task__via_send_message_persists_text_column():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "via send_message")

    sm = broker.get_sync_sessionmaker()
    with sm() as s:
        row = s.execute(
            text("SELECT text FROM tasks WHERE task_id = :tid"),
            {"tid": sent["task"]["task_id"]},
        ).fetchone()
    assert row is not None
    assert row[0] == "via send_message"


# ---------------------------------------------------------------------------
# 4. _read_task — flat typed-column dict (no json.loads)
# ---------------------------------------------------------------------------


def test_read_task__returns_flat_typed_dict():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "read me back")
    tid = sent["task"]["task_id"]

    sm = broker.get_sync_sessionmaker()
    with sm() as s:
        result = broker._read_task(s, tid)
    assert result is not None
    _assert_flat_typed_shape(result, expect_type="unicast")
    assert result["text"] == "read me back"


def test_read_task__returns_none_for_missing_task():
    sm = broker.get_sync_sessionmaker()
    with sm() as s:
        result = broker._read_task(s, str(uuid.uuid4()))
    assert result is None


# ---------------------------------------------------------------------------
# 5. poll_tasks — typed-column dicts; still filters broadcast_summary
# ---------------------------------------------------------------------------


def test_poll_tasks__returns_flat_typed_task_dicts():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "Hello")

    result = broker.poll_tasks(recipient)
    assert len(result) == 1
    _assert_flat_typed_shape(result[0], expect_type="unicast")


def test_poll_tasks__no_metadata_or_artifacts_wrapping():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "Hello")

    [task] = broker.poll_tasks(recipient)
    assert "metadata" not in task
    assert "artifacts" not in task
    assert "history" not in task


def test_poll_tasks__text_field_holds_body():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "specific body")

    [task] = broker.poll_tasks(recipient)
    assert task["text"] == "specific body"


def test_poll_tasks__still_filters_broadcast_summary():
    sid, sender, _b_id, _c_id = _setup_three_agents()
    broker.broadcast_message(sid, sender, "broadcast body")

    sender_tasks = broker.poll_tasks(sender)
    types = [t["type"] for t in sender_tasks]
    assert "broadcast_summary" not in types


def test_poll_tasks__since_filter_still_works_post_surface_14():
    sid, sender, recipient = _setup_two_agents()
    sent_first = broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")

    cutoff = sent_first["task"]["status_timestamp"]
    later = broker.poll_tasks(recipient, since=cutoff)
    later_texts = {t["text"] for t in later}
    assert "second" in later_texts


# ---------------------------------------------------------------------------
# 6. ack_task — typed-column round-trip; status_state updated
# ---------------------------------------------------------------------------


def test_ack_task__returns_flat_typed_dict_with_completed_state():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "ack me")
    tid = sent["task"]["task_id"]

    result = broker.ack_task(recipient, tid)
    assert "task" in result
    _assert_flat_typed_shape(result["task"], expect_type="unicast")
    assert result["task"]["status_state"] == "completed"


def test_ack_task__round_trip_preserves_text_via_typed_columns():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "round trip ack")
    tid = sent["task"]["task_id"]

    broker.ack_task(recipient, tid)
    [task] = broker.poll_tasks(recipient, status="completed")
    assert task["text"] == "round trip ack"
    assert task["status_state"] == "completed"


def test_ack_task__rejects_when_not_recipient():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "ack me")
    tid = sent["task"]["task_id"]

    with pytest.raises(PermissionError):
        broker.ack_task(sender, tid)


# ---------------------------------------------------------------------------
# 7. cancel_task — typed-column round-trip; status_state updated
# ---------------------------------------------------------------------------


def test_cancel_task__returns_flat_typed_dict_with_canceled_state():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "cancel me")
    tid = sent["task"]["task_id"]

    result = broker.cancel_task(sender, tid)
    assert "task" in result
    _assert_flat_typed_shape(result["task"], expect_type="unicast")
    assert result["task"]["status_state"] == "canceled"


def test_cancel_task__round_trip_preserves_text_via_typed_columns():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "round trip cancel")
    tid = sent["task"]["task_id"]

    broker.cancel_task(sender, tid)
    [task] = broker.poll_tasks(recipient, status="canceled")
    assert task["text"] == "round trip cancel"
    assert task["status_state"] == "canceled"


def test_cancel_task__rejects_when_not_sender():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "cancel me")
    tid = sent["task"]["task_id"]

    with pytest.raises(PermissionError):
        broker.cancel_task(recipient, tid)


# ---------------------------------------------------------------------------
# 8. broadcast_message + summary builder — flat shape, to_agent_id=''
# ---------------------------------------------------------------------------


def test_broadcast_message__summary_is_flat_typed_shape():
    sid, sender, _b, _c = _setup_three_agents()
    [result] = broker.broadcast_message(sid, sender, "Attention all")
    _assert_flat_typed_shape(result["task"], expect_type="broadcast_summary")


def test_broadcast_message__summary_no_metadata_or_artifacts_wrapping():
    sid, sender, _b, _c = _setup_three_agents()
    [result] = broker.broadcast_message(sid, sender, "Attention all")
    summary = result["task"]
    assert "metadata" not in summary
    assert "artifacts" not in summary
    assert "history" not in summary


def test_broadcast_message__summary_to_agent_id_is_empty_string():
    """Per Surface 14: builder passes ``to_agent_id=''`` explicitly, no .get fallback."""
    sid, sender, _b, _c = _setup_three_agents()
    [result] = broker.broadcast_message(sid, sender, "anyone")
    summary = result["task"]
    assert summary["to_agent_id"] == ""


def test_broadcast_message__summary_from_agent_id_is_sender():
    sid, sender, _b, _c = _setup_three_agents()
    [result] = broker.broadcast_message(sid, sender, "anyone")
    assert result["task"]["from_agent_id"] == sender


def test_broadcast_message__summary_context_id_is_sender():
    sid, sender, _b, _c = _setup_three_agents()
    [result] = broker.broadcast_message(sid, sender, "anyone")
    assert result["task"]["context_id"] == sender


def test_broadcast_message__summary_text_describes_recipient_count():
    sid, sender, _b, _c = _setup_three_agents()
    [result] = broker.broadcast_message(sid, sender, "anyone?")
    assert "Broadcast sent" in result["task"]["text"]


def test_broadcast_message__summary_status_state_is_completed():
    sid, sender, _b, _c = _setup_three_agents()
    [result] = broker.broadcast_message(sid, sender, "anyone")
    assert result["task"]["status_state"] == "completed"


def test_broadcast_message__delivery_tasks_are_flat_typed_shape():
    sid, sender, b_id, _c = _setup_three_agents()
    broker.broadcast_message(sid, sender, "delivery body")

    [delivered] = broker.poll_tasks(b_id)
    _assert_flat_typed_shape(delivered, expect_type="unicast")
    assert delivered["text"] == "delivery body"


def test_broadcast_message__delivery_origin_task_id_links_to_summary():
    sid, sender, b_id, _c = _setup_three_agents()
    [result] = broker.broadcast_message(sid, sender, "delivery body")
    summary_id = result["task"]["task_id"]

    [delivered] = broker.poll_tasks(b_id)
    assert delivered["origin_task_id"] == summary_id


def test_broadcast_message__notifications_sent_count_at_wrapper_level():
    sid, sender, _b, _c = _setup_three_agents()
    [result] = broker.broadcast_message(sid, sender, "anyone")
    assert "notifications_sent_count" in result
    assert isinstance(result["notifications_sent_count"], int)


# ---------------------------------------------------------------------------
# 9. _list_tasks_where (via list_inbox / list_sent)
# ---------------------------------------------------------------------------


def test_list_inbox__returns_flat_typed_dicts_no_task_json_field():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "inbox body")

    rows = broker.list_inbox(recipient)
    assert len(rows) == 1
    row = rows[0]
    for k in REQUIRED_TASK_KEYS:
        assert k in row, f"missing typed-column key {k} in list_inbox row"
    assert "task_json" not in row, "list_inbox row should not carry a task_json field"
    assert row["text"] == "inbox body"


def test_list_sent__returns_flat_typed_dicts_no_task_json_field():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "sent body")

    rows = broker.list_sent(sender)
    assert len(rows) == 1
    row = rows[0]
    for k in REQUIRED_TASK_KEYS:
        assert k in row, f"missing typed-column key {k} in list_sent row"
    assert "task_json" not in row, "list_sent row should not carry a task_json field"
    assert row["text"] == "sent body"


def test_list_inbox__filters_out_broadcast_summary():
    sid, sender, _b, _c = _setup_three_agents()
    broker.broadcast_message(sid, sender, "broadcast body")

    sent_rows = broker.list_sent(sender)
    types = [r["type"] for r in sent_rows]
    assert "broadcast_summary" not in types


# ---------------------------------------------------------------------------
# 10. list_timeline — no json.loads; typed-column shape
# ---------------------------------------------------------------------------


def _timeline_task(row: dict) -> dict:
    """Return the per-row task dict regardless of whether list_timeline wraps
    rows under ``task`` or returns flat rows directly."""
    candidate = row.get("task") if isinstance(row.get("task"), dict) else row
    assert isinstance(candidate, dict)
    return candidate


def test_list_timeline__entries_have_no_metadata_or_artifacts_wrapping():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "timeline body")

    rows = broker.list_timeline(sid)
    assert len(rows) >= 1
    for row in rows:
        task = _timeline_task(row)
        assert "metadata" not in task
        assert "artifacts" not in task
        assert "history" not in task
        assert "task_json" not in task


def test_list_timeline__entries_expose_text_via_typed_column():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "timeline-readable")

    rows = broker.list_timeline(sid)
    bodies = []
    for row in rows:
        task = _timeline_task(row)
        if "text" in task:
            bodies.append(task["text"])
        elif "text" in row:
            bodies.append(row["text"])
    assert "timeline-readable" in bodies


def test_list_timeline__filters_out_broadcast_summary():
    sid, sender, _b, _c = _setup_three_agents()
    broker.broadcast_message(sid, sender, "broadcast body")

    rows = broker.list_timeline(sid)
    types = []
    for row in rows:
        task = _timeline_task(row)
        if "type" in task:
            types.append(task["type"])
    assert "broadcast_summary" not in types


# ---------------------------------------------------------------------------
# 11. get_task — typed-column shape, no metadata wrapping
# ---------------------------------------------------------------------------


def test_get_task__returns_flat_typed_task():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "fetch me")
    tid = sent["task"]["task_id"]

    result = broker.get_task(sid, tid)
    assert "task" in result
    _assert_flat_typed_shape(result["task"], expect_type="unicast")
    assert result["task"]["text"] == "fetch me"


def test_get_task__no_metadata_inspection_after_surface_14():
    """get_task must not access task_dict['metadata']['fromAgentId'] etc.

    Post-Surface-14, sender / recipient identifiers live at the top of the
    flat dict; the body is at ``text``. We assert the call succeeds and the
    return shape carries the typed columns, which together force any caller
    that previously dereferenced ``metadata`` to be rewritten.
    """
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "fetch me")
    tid = sent["task"]["task_id"]

    result = broker.get_task(sid, tid)
    task = result["task"]
    assert "metadata" not in task
    assert "artifacts" not in task
    assert task["from_agent_id"] == sender
    assert task["to_agent_id"] == recipient


def test_get_task__rejects_task_not_in_session():
    """A task whose endpoints don't live in the session yields ValueError."""
    sid, sender, recipient = _setup_two_agents()
    other_session = _create_session()
    other_sid = other_session["session_id"]
    other_sender = _register_agent(other_sid, name="outsider-sender")
    other_recipient = _register_agent(other_sid, name="outsider-recipient")
    sent = broker.send_message(
        other_sid,
        other_sender["agent_id"],
        other_recipient["agent_id"],
        "elsewhere",
    )

    with pytest.raises(ValueError, match="not found"):
        broker.get_task(sid, sent["task"]["task_id"])
