"""Broker-level session bootstrap tests (design doc 0000026).

Per principle (ii)/(iii) of design 0000061: per-API parametrized "shape +
behaviour" pairs covering bootstrap, rollback, delete cascade, soft-delete
filtering, and director protection.
"""

import uuid
from unittest.mock import Mock

import click
import pytest

from cafleet import broker
from cafleet.broker import _is_administrator
from cafleet.db.models import (
    Agent,
    AgentPlacement,
    Task,
)
from cafleet.db.models import (
    Session as SessionModel,
)
from cafleet.tmux import DirectorContext


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


@pytest.fixture
def director_context():
    return DirectorContext(session="main", window_id="@1", pane_id="%0")


def _bootstrap(label=None, ctx=None, coding_agent="claude"):
    return broker.create_session(
        label=label,
        director_context=ctx or DirectorContext(session="main", window_id="@1", pane_id="%0"),
        coding_agent=coding_agent,
    )


def test_bootstrap__top_level_envelope_and_director_subdict(director_context):
    result = _bootstrap(label="bootstrap-1", ctx=director_context)
    for key in ("session_id", "label", "created_at", "administrator_agent_id", "director"):
        assert key in result
    director = result["director"]
    for key in ("agent_id", "name", "description", "registered_at", "placement"):
        assert key in director
    assert director["name"] == "Director"
    assert director["description"] == "Root Director for this session"
    assert result["label"] == "bootstrap-1"


@pytest.mark.parametrize("coding_agent", ["claude", "codex"])
def test_bootstrap__placement_matches_director_context_and_records_coding_agent(
    director_context, coding_agent
):
    result = _bootstrap(ctx=director_context, coding_agent=coding_agent)
    placement = result["director"]["placement"]
    assert placement["director_agent_id"] is None
    assert placement["tmux_session"] == director_context.session
    assert placement["tmux_window_id"] == director_context.window_id
    assert placement["tmux_pane_id"] == director_context.pane_id
    assert placement["coding_agent"] == coding_agent
    assert "created_at" in placement


def test_bootstrap__db_rows_for_session_director_administrator_placement(
    broker_session, director_context
):
    result = _bootstrap(ctx=director_context)
    sid = result["session_id"]

    with broker_session() as s:
        session_rows = s.query(SessionModel).all()
        agent_rows = s.query(Agent).filter(Agent.session_id == sid).all()
        placement_rows = s.query(AgentPlacement).all()

    assert len(session_rows) == 1
    assert session_rows[0].session_id == sid
    assert session_rows[0].director_agent_id == result["director"]["agent_id"]
    assert session_rows[0].deleted_at is None

    assert len(agent_rows) == 2
    by_name = {r.name: r for r in agent_rows}
    assert by_name["Director"].status == "active"
    assert by_name["Administrator"].status == "active"
    assert by_name["Director"].agent_id == result["director"]["agent_id"]
    assert by_name["Administrator"].agent_id == result["administrator_agent_id"]
    assert _is_administrator(by_name["Administrator"].agent_card_json)
    assert not _is_administrator(by_name["Director"].agent_card_json)

    assert len(placement_rows) == 1
    placement = placement_rows[0]
    assert placement.agent_id == result["director"]["agent_id"]
    assert placement.director_agent_id is None
    assert placement.tmux_pane_id == director_context.pane_id

    # Administrator.registered_at == sessions.created_at.
    assert by_name["Administrator"].registered_at == session_rows[0].created_at


@pytest.mark.parametrize("label", ["hello-world", None])
def test_bootstrap__label_handling_and_unique_ids(director_context, label):
    r1 = _bootstrap(label=label, ctx=director_context)
    r2 = _bootstrap(label=label, ctx=director_context)
    assert r1["label"] == label
    assert r1["session_id"] != r2["session_id"]
    assert r1["director"]["agent_id"] != r2["director"]["agent_id"]
    assert r1["administrator_agent_id"] != r2["administrator_agent_id"]


def test_bootstrap__atomic_rollback_on_failure(broker_session, director_context, monkeypatch):
    class _BoomPlacement:
        def __init__(self, *args, **kwargs):
            raise RuntimeError("injected failure after INSERT agents")

    monkeypatch.setattr(broker, "AgentPlacement", _BoomPlacement)
    with pytest.raises(RuntimeError, match="injected failure"):
        broker.create_session(
            label="rollback", director_context=director_context, coding_agent="claude",
        )

    with broker_session() as s:
        assert s.query(SessionModel).count() == 0
        assert s.query(Agent).count() == 0
        assert s.query(AgentPlacement).count() == 0


def test_delete_session__soft_deletes_deregisters_and_drops_placement(
    broker_session, director_context
):
    result = _bootstrap(ctx=director_context)
    sid = result["session_id"]
    admin_id = result["administrator_agent_id"]
    director_id = result["director"]["agent_id"]
    sent = broker.send_message(sid, admin_id, director_id, "audit me")
    task_id = sent["task"]["task_id"]

    ret = broker.delete_session(sid)
    assert ret["deregistered_count"] == 2

    with broker_session() as s:
        session_row = s.query(SessionModel).filter(SessionModel.session_id == sid).one()
        statuses = {
            r.name: r.status
            for r in s.query(Agent).filter(Agent.session_id == sid).all()
        }
        placement_count = s.query(AgentPlacement).count()
        tasks = s.query(Task).all()
    assert session_row.deleted_at is not None
    assert statuses["Director"] == "deregistered"
    assert statuses["Administrator"] == "deregistered"
    assert placement_count == 0
    # Tasks preserved across soft-delete.
    assert any(t.task_id == task_id for t in tasks)


def test_delete_session__idempotent_rerun_returns_zero(director_context):
    result = _bootstrap(ctx=director_context)
    sid = result["session_id"]
    first = broker.delete_session(sid)
    second = broker.delete_session(sid)
    assert first["deregistered_count"] == 2
    assert second["deregistered_count"] == 0


def test_delete_session__unknown_session_raises_click_exception():
    fake_sid = str(uuid.uuid4())
    with pytest.raises(click.ClickException) as exc_info:
        broker.delete_session(fake_sid)
    msg = str(exc_info.value)
    assert "not found" in msg.lower()
    assert fake_sid in msg


@pytest.mark.parametrize(
    ("scenario", "expected_substring", "must_not_contain"),
    [
        ("soft_deleted_session", "is deleted", "not found"),
        ("unknown_session", "not found", None),
    ],
)
def test_register_agent__rejects_dead_sessions(
    director_context, scenario, expected_substring, must_not_contain
):
    if scenario == "soft_deleted_session":
        result = _bootstrap(ctx=director_context)
        sid = result["session_id"]
        broker.delete_session(sid)
    else:
        sid = str(uuid.uuid4())

    with pytest.raises(click.UsageError) as exc_info:
        broker.register_agent(
            session_id=sid, name="late-comer", description="too late",
        )
    msg = str(exc_info.value)
    assert expected_substring in msg.lower() if scenario == "unknown_session" else expected_substring in msg
    if must_not_contain is not None:
        assert must_not_contain not in msg.lower()


def test_list_sessions__hides_soft_deleted_but_get_session_still_returns(director_context):
    alive = _bootstrap(label="alive", ctx=director_context)
    dead = _bootstrap(label="dead", ctx=director_context)
    broker.delete_session(dead["session_id"])

    sessions = broker.list_sessions()
    ids = {s["session_id"] for s in sessions}
    assert alive["session_id"] in ids
    assert dead["session_id"] not in ids

    # get_session still returns the soft-deleted row.
    row = broker.get_session(dead["session_id"])
    assert row is not None
    assert row["deleted_at"] is not None


def test_deregister_agent__root_director_protected_non_root_unaffected(
    broker_session, director_context
):
    result = _bootstrap(ctx=director_context)
    sid = result["session_id"]
    director_id = result["director"]["agent_id"]

    with pytest.raises(click.UsageError) as exc_info:
        broker.deregister_agent(director_id)
    msg = str(exc_info.value)
    assert "cannot deregister the root Director" in msg
    assert "cafleet session delete" in msg

    # State unchanged.
    with broker_session() as s:
        d_row = s.query(Agent).filter(Agent.agent_id == director_id).one()
        p_row = (
            s.query(AgentPlacement).filter(AgentPlacement.agent_id == director_id).one()
        )
        sess_row = s.query(SessionModel).filter(SessionModel.session_id == sid).one()
    assert d_row.status == "active"
    assert d_row.deregistered_at is None
    assert p_row.tmux_pane_id == director_context.pane_id
    assert sess_row.director_agent_id == director_id

    # Non-root member can still be deregistered.
    member = broker.register_agent(
        session_id=sid, name="regular-member", description="regular",
    )
    assert broker.deregister_agent(member["agent_id"]) is True


def test_send_message__notification_invokes_inline_preview_with_director_pane(
    director_context, monkeypatch
):
    mock_preview = Mock(return_value=True)
    monkeypatch.setattr("cafleet.tmux.send_inline_preview", mock_preview)
    result = broker.create_session(
        label="notify", director_context=director_context, coding_agent="claude",
    )
    sid = result["session_id"]
    root_director_id = result["director"]["agent_id"]
    member = broker.register_agent(
        session_id=sid, name="member", description="member under root",
        placement={
            "director_agent_id": root_director_id,
            "tmux_session": "main", "tmux_window_id": "@1",
            "tmux_pane_id": "%1", "coding_agent": "claude",
        },
    )
    response = broker.send_message(
        sid, member["agent_id"], to=root_director_id, text="hi director",
    )
    assert response["notification_sent"] is True
    assert mock_preview.call_count == 1
    kwargs = mock_preview.call_args.kwargs
    assert kwargs["target_pane_id"] == director_context.pane_id
    assert kwargs["sender_8"] == member["agent_id"][:8]
    assert kwargs["text"] == "hi director"
