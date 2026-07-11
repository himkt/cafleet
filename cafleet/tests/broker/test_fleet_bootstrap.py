"""Broker-level fleet bootstrap tests."""

from unittest.mock import Mock

import click
import pytest

from cafleet import broker
from cafleet.broker import fleets
from cafleet.broker._shared import is_administrator
from cafleet.db.models import (
    Fleet as FleetModel,
)
from cafleet.db.models import (
    Member,
    MemberPlacement,
    Task,
)
from cafleet.multiplexer import MultiplexerContext as DirectorContext


@pytest.fixture
def director_context():
    return DirectorContext(session="main", window_id="@1", pane_id="%0")


def _bootstrap(name=None, ctx=None, coding_agent="claude", backend="tmux"):
    return broker.create_fleet(
        name=name,
        director_context=ctx
        or DirectorContext(session="main", window_id="@1", pane_id="%0"),
        coding_agent=coding_agent,
        backend=backend,
    )


def test_bootstrap__top_level_envelope_and_director_subdict(director_context):
    result = _bootstrap(name="bootstrap-1", ctx=director_context)
    for key in (
        "fleet_id",
        "name",
        "created_at",
        "administrator_member_id",
        "director",
    ):
        assert key in result
    director = result["director"]
    for key in ("member_id", "name", "description", "registered_at", "placement"):
        assert key in director
    assert director["name"] == "Director"
    assert director["description"] == "Root Director for this fleet"
    assert result["name"] == "bootstrap-1"


@pytest.mark.parametrize("coding_agent", ["claude", "codex"])
def test_bootstrap__placement_matches_director_context_and_records_coding_agent(
    director_context, coding_agent
):
    result = _bootstrap(ctx=director_context, coding_agent=coding_agent)
    placement = result["director"]["placement"]
    assert placement["backend"] == "tmux"
    assert placement["mux_session"] == director_context.session
    assert placement["mux_window_id"] == director_context.window_id
    assert placement["mux_pane_id"] == director_context.pane_id
    assert placement["coding_agent"] == coding_agent
    assert "created_at" in placement


def test_bootstrap__db_rows_for_fleet_director_administrator_placement(
    broker_session, director_context
):
    result = _bootstrap(ctx=director_context)
    sid = result["fleet_id"]

    with broker_session() as s:
        fleet_rows = s.query(FleetModel).all()
        member_rows = s.query(Member).filter(Member.fleet_id == sid).all()
        placement_rows = s.query(MemberPlacement).all()

    assert len(fleet_rows) == 1
    assert fleet_rows[0].fleet_id == sid
    assert fleet_rows[0].director_member_id == result["director"]["member_id"]
    assert fleet_rows[0].deleted_at is None

    assert len(member_rows) == 2
    by_name = {r.name: r for r in member_rows}
    assert by_name["Director"].status == "active"
    assert by_name["Administrator"].status == "active"
    assert by_name["Director"].member_id == result["director"]["member_id"]
    assert by_name["Administrator"].member_id == result["administrator_member_id"]
    assert is_administrator(by_name["Administrator"].member_card_json)
    assert not is_administrator(by_name["Director"].member_card_json)

    assert len(placement_rows) == 1
    placement = placement_rows[0]
    assert placement.member_id == result["director"]["member_id"]
    assert placement.mux_pane_id == director_context.pane_id

    # Administrator.registered_at == fleets.created_at.
    assert by_name["Administrator"].registered_at == fleet_rows[0].created_at


@pytest.mark.parametrize("name", ["hello-world", None])
def test_bootstrap__name_handling_and_unique_ids(director_context, name):
    r1 = _bootstrap(name=name, ctx=director_context)
    r2 = _bootstrap(name=name, ctx=director_context)
    assert r1["name"] == name
    assert r1["fleet_id"] != r2["fleet_id"]
    assert r1["director"]["member_id"] != r2["director"]["member_id"]
    assert r1["administrator_member_id"] != r2["administrator_member_id"]


def test_bootstrap__atomic_rollback_on_failure(
    broker_session, director_context, monkeypatch
):
    class _BoomPlacement:
        def __init__(self, *args, **kwargs):
            raise RuntimeError("injected failure after INSERT members")

    monkeypatch.setattr(fleets, "MemberPlacement", _BoomPlacement)
    with pytest.raises(RuntimeError, match="injected failure"):
        broker.create_fleet(
            name="rollback",
            director_context=director_context,
            coding_agent="claude",
            backend="tmux",
        )

    with broker_session() as s:
        assert s.query(FleetModel).count() == 0
        assert s.query(Member).count() == 0
        assert s.query(MemberPlacement).count() == 0


def test_delete_fleet__soft_deletes_deregisters_and_drops_placement(
    broker_session, director_context
):
    result = _bootstrap(ctx=director_context)
    sid = result["fleet_id"]
    admin_id = result["administrator_member_id"]
    director_id = result["director"]["member_id"]
    sent = broker.send_message(sid, admin_id, director_id, "audit me")
    task_id = sent["task"]["task_id"]

    ret = broker.delete_fleet(sid)
    assert ret["deregistered_count"] == 2

    with broker_session() as s:
        fleet_row = s.query(FleetModel).filter(FleetModel.fleet_id == sid).one()
        statuses = {
            r.name: r.status
            for r in s.query(Member).filter(Member.fleet_id == sid).all()
        }
        placement_count = s.query(MemberPlacement).count()
        tasks = s.query(Task).all()
    assert fleet_row.deleted_at is not None
    assert statuses["Director"] == "deregistered"
    assert statuses["Administrator"] == "deregistered"
    assert placement_count == 0
    # Tasks preserved across soft-delete.
    assert any(t.task_id == task_id for t in tasks)


def test_delete_fleet__idempotent_rerun_returns_zero(director_context):
    result = _bootstrap(ctx=director_context)
    sid = result["fleet_id"]
    first = broker.delete_fleet(sid)
    second = broker.delete_fleet(sid)
    assert first["deregistered_count"] == 2
    assert second["deregistered_count"] == 0


def test_delete_fleet__unknown_fleet_raises_click_exception():
    fake_sid = 999999
    with pytest.raises(click.ClickException) as exc_info:
        broker.delete_fleet(fake_sid)
    msg = str(exc_info.value)
    assert "not found" in msg.lower()
    assert str(fake_sid) in msg


@pytest.mark.parametrize(
    ("scenario", "expected_substring", "must_not_contain", "expected_exit_code"),
    [
        # Soft-deleted fleet exits 1 (ClickException); unknown fleet stays exit 2
        # (UsageError) — item 3.3 normalizes only the soft-deleted guard.
        ("soft_deleted_fleet", "is deleted", "not found", 1),
        ("unknown_fleet", "not found", None, 2),
    ],
)
def test_register_member__rejects_dead_fleets(
    director_context,
    scenario,
    expected_substring,
    must_not_contain,
    expected_exit_code,
):
    if scenario == "soft_deleted_fleet":
        result = _bootstrap(ctx=director_context)
        sid = result["fleet_id"]
        broker.delete_fleet(sid)
    else:
        sid = 999999

    with pytest.raises(click.ClickException) as exc_info:
        broker.register_member(
            fleet_id=sid,
            name="late-comer",
            description="too late",
        )
    assert exc_info.value.exit_code == expected_exit_code
    msg = str(exc_info.value)
    assert (
        expected_substring in msg.lower()
        if scenario == "unknown_fleet"
        else expected_substring in msg
    )
    if must_not_contain is not None:
        assert must_not_contain not in msg.lower()


def test_list_fleets__hides_soft_deleted_but_get_fleet_still_returns(
    director_context,
):
    alive = _bootstrap(name="alive", ctx=director_context)
    dead = _bootstrap(name="dead", ctx=director_context)
    broker.delete_fleet(dead["fleet_id"])

    fleets = broker.list_fleets()
    ids = {s["fleet_id"] for s in fleets}
    assert alive["fleet_id"] in ids
    assert dead["fleet_id"] not in ids

    # get_fleet still returns the soft-deleted row.
    row = broker.get_fleet(dead["fleet_id"])
    assert row is not None
    assert row["deleted_at"] is not None


def test_deregister_member__root_director_protected_non_root_unaffected(
    broker_session, director_context
):
    result = _bootstrap(ctx=director_context)
    sid = result["fleet_id"]
    director_id = result["director"]["member_id"]

    with pytest.raises(click.ClickException) as exc_info:
        broker.deregister_member(director_id)
    assert exc_info.value.exit_code == 1
    msg = str(exc_info.value)
    assert "cannot deregister the root Director" in msg
    assert "cafleet fleet delete" in msg

    # State unchanged.
    with broker_session() as s:
        d_row = s.query(Member).filter(Member.member_id == director_id).one()
        p_row = (
            s.query(MemberPlacement)
            .filter(MemberPlacement.member_id == director_id)
            .one()
        )
        sess_row = s.query(FleetModel).filter(FleetModel.fleet_id == sid).one()
    assert d_row.status == "active"
    assert d_row.deregistered_at is None
    assert p_row.mux_pane_id == director_context.pane_id
    assert sess_row.director_member_id == director_id

    # Non-root member can still be deregistered.
    member = broker.register_member(
        fleet_id=sid,
        name="regular-member",
        description="regular",
    )
    assert broker.deregister_member(member["member_id"]) is True


def test_send_message__notification_invokes_inline_preview_with_director_pane(
    director_context, monkeypatch
):
    mock_preview = Mock(return_value=True)
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.send_inline_preview",
        lambda self, **kwargs: mock_preview(**kwargs),
    )
    result = broker.create_fleet(
        name="notify",
        director_context=director_context,
        coding_agent="claude",
        backend="tmux",
    )
    sid = result["fleet_id"]
    root_director_id = result["director"]["member_id"]
    member = broker.register_member(
        fleet_id=sid,
        name="member",
        description="member under root",
        placement={
            "backend": "tmux",
            "mux_session": "main",
            "mux_window_id": "@1",
            "mux_pane_id": "%1",
            "coding_agent": "claude",
        },
    )
    response = broker.send_message(
        sid,
        member["member_id"],
        to=root_director_id,
        text="hi director",
    )
    assert response["notification_sent"] is True
    assert mock_preview.call_count == 1
    kwargs = mock_preview.call_args.kwargs
    assert kwargs["target_pane_id"] == director_context.pane_id
    assert kwargs["sender_id"] == member["member_id"]
    assert kwargs["text"] == "hi director"
