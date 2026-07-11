"""Tests for ``broker`` fleet + registry operations."""

import click
import pytest

from cafleet import broker
from cafleet.broker import _shared
from cafleet.db.models import Member
from tests.broker._helpers import _create_fleet, _member_placement, _register_member

# --- create_fleet ------------------------------------------------------


def test_create_fleet__shape_and_name_handling():
    r_default = _create_fleet()
    assert {"fleet_id", "name", "created_at"}.issubset(r_default.keys())
    assert isinstance(r_default["fleet_id"], int)
    assert r_default["name"] is None
    assert "T" in r_default["created_at"]

    r_named = _create_fleet(name="PR-42 review")
    assert r_named["name"] == "PR-42 review"
    assert r_named["fleet_id"] != r_default["fleet_id"]


def test_create_fleet__seeds_only_the_root_director(broker_session):
    r1 = _create_fleet()
    sid = r1["fleet_id"]

    with broker_session() as s:
        rows = (
            s.query(Member)
            .filter(Member.fleet_id == sid, Member.status == "active")
            .all()
        )
    assert len(rows) == 1
    assert rows[0].name == "Director"
    assert rows[0].member_id == r1["director"]["member_id"]


def test_create_fleet__roster_marks_director_and_member_kinds():
    result = _create_fleet()
    sid = result["fleet_id"]
    _register_member(sid, name="user-member")
    entries = broker.list_roster(sid)
    assert len(entries) == 2
    kinds = {e["member_id"]: e["kind"] for e in entries}
    assert kinds[result["director"]["member_id"]] == "director"
    member_entries = [e for e in entries if e["kind"] == "member"]
    assert {e["name"] for e in member_entries} == {"user-member"}


# --- list_fleets -------------------------------------------------------


def test_list_fleets__empty_and_non_empty_with_member_count():
    assert broker.list_fleets() == []

    fleet = _create_fleet(name="fleet-a")
    sid = fleet["fleet_id"]
    _register_member(sid, name="member-1")
    _register_member(sid, name="member-2")
    dead = _register_member(sid, name="dead-member")
    broker.deregister_member(dead["member_id"])

    rows = broker.list_fleets()
    assert len(rows) == 1
    row = rows[0]
    assert set(row.keys()) >= {"fleet_id", "name", "created_at", "member_count"}
    assert row["name"] == "fleet-a"
    # Two user members (one deregistered → excluded) + Director.
    assert row["member_count"] == 3


def test_list_fleets__bootstrap_only_count_is_one():
    _create_fleet()
    rows = broker.list_fleets()
    assert rows[0]["member_count"] == 1


def test_list_fleets__newest_first_by_created_at_desc(monkeypatch):
    # Force distinct, ascending timestamps so the assertion does not depend
    # on microsecond clock resolution. Iterator yields one per create_fleet() call.
    timestamps = iter(
        [
            "2026-05-23T00:00:01.000000+00:00",
            "2026-05-23T00:00:02.000000+00:00",
            "2026-05-23T00:00:03.000000+00:00",
        ]
    )
    monkeypatch.setattr(_shared, "now_iso", lambda: next(timestamps))

    _create_fleet(name="a")  # 00:00:01
    _create_fleet(name="b")  # 00:00:02
    _create_fleet(name="c")  # 00:00:03

    rows = broker.list_fleets()
    names = [row["name"] for row in rows]

    assert names == ["c", "b", "a"]


# --- get_fleet ---------------------------------------------------------


def test_get_fleet__existing_and_nonexistent():
    created = _create_fleet(name="find-me")
    found = broker.get_fleet(created["fleet_id"])
    assert found is not None
    assert found["fleet_id"] == created["fleet_id"]
    assert found["name"] == "find-me"
    assert "created_at" in found

    assert broker.get_fleet(999999) is None


# --- register_member ------------------------------------------------------


def test_register_member__shape_and_unique_id():
    fleet = _create_fleet()
    r1 = _register_member(fleet["fleet_id"], name="a1")
    r2 = _register_member(fleet["fleet_id"], name="a2")
    assert {"member_id", "name", "registered_at"}.issubset(r1.keys())
    assert isinstance(r1["member_id"], int)
    assert r1["name"] == "a1"
    assert r1["member_id"] != r2["member_id"]


def test_register_member__missing_fleet_is_usage_error():
    with pytest.raises(click.UsageError, match="not found"):
        broker.register_member(
            fleet_id=999999,
            name="orphan",
            description="no fleet",
        )


def test_register_member__inactive_root_director_invariant_guard(broker_session):
    """A placed registration requires the fleet's root Director to be active —
    a loud invariant failure; the Director id is resolved from the fleet row,
    not caller input."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]

    with broker_session() as s:
        s.query(Member).filter(Member.member_id == director_id).update(
            {"status": "deregistered"}
        )
        s.commit()

    with pytest.raises(click.ClickException, match="is not active"):
        _register_member(sid, name="late-member", placement=_member_placement(None))


@pytest.mark.parametrize("with_placement", [True, False])
def test_register_member__placement_stored_or_absent(with_placement):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    if with_placement:
        member = _register_member(sid, name="member", placement=_member_placement(None))
        fetched = broker.get_member(member["member_id"], sid)
        assert fetched["placement"] is not None
        assert fetched["placement"]["mux_session"] == "main"
    else:
        member = _register_member(sid, name="standalone")
        fetched = broker.get_member(member["member_id"], sid)
        assert fetched["placement"] is None


# --- get_member -----------------------------------------------------------


def test_get_member__returns_typed_envelope():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    member = _register_member(sid, name="visible", description="test desc")
    result = broker.get_member(member["member_id"], sid)
    assert result["member_id"] == member["member_id"]
    assert result["name"] == "visible"
    assert result["description"] == "test desc"
    assert result["status"] == "active"
    assert "registered_at" in result


@pytest.mark.parametrize(
    "scenario",
    ["nonexistent_member", "deregistered_member", "wrong_fleet"],
)
def test_get_member__returns_none_for_missing(scenario):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    if scenario == "nonexistent_member":
        assert broker.get_member(999999, sid) is None
    elif scenario == "deregistered_member":
        member = _register_member(sid, name="temp")
        broker.deregister_member(member["member_id"])
        assert broker.get_member(member["member_id"], sid) is None
    else:
        other = _create_fleet()
        member = _register_member(sid, name="scoped")
        assert broker.get_member(member["member_id"], other["fleet_id"]) is None


# --- list_roster ---------------------------------------------------------


def test_list_roster__active_only_with_required_keys():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    _register_member(sid, name="active-1")
    _register_member(sid, name="active-2")
    dead = _register_member(sid, name="dead-member")
    broker.deregister_member(dead["member_id"])

    result = broker.list_roster(sid)
    assert len(result) == 3  # 2 user + Director
    names = {a["name"] for a in result}
    assert names == {"active-1", "active-2", "Director"}
    entry = result[0]
    assert {
        "member_id",
        "name",
        "description",
        "status",
        "registered_at",
        "kind",
    }.issubset(entry.keys())
    assert entry["status"] == "active"


def test_list_roster__bootstrap_only_lists_the_director():
    fleet = _create_fleet()
    result = broker.list_roster(fleet["fleet_id"])
    assert {a["name"] for a in result} == {"Director"}


def test_list_roster__scoped_per_fleet():
    fleet_a = _create_fleet()
    fleet_b = _create_fleet()
    _register_member(fleet_a["fleet_id"], name="member-a")
    _register_member(fleet_b["fleet_id"], name="member-b")
    result_a = broker.list_roster(fleet_a["fleet_id"])
    names_a = {a["name"] for a in result_a}
    assert "member-a" in names_a
    assert "member-b" not in names_a


# --- verify_member_fleet ------------------------------------------------


@pytest.mark.parametrize(
    ("scenario", "expected"),
    [
        ("member_in_fleet", True),
        ("member_in_different_fleet", False),
        ("nonexistent_member", False),
    ],
)
def test_verify_member_fleet__matrix(scenario, expected):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    if scenario == "member_in_fleet":
        member = _register_member(sid, name="here")
        assert broker.verify_member_fleet(member["member_id"], sid) is expected
    elif scenario == "member_in_different_fleet":
        other = _create_fleet()
        member = _register_member(sid, name="there")
        assert (
            broker.verify_member_fleet(member["member_id"], other["fleet_id"])
            is expected
        )
    else:
        assert broker.verify_member_fleet(999999, sid) is expected


# --- deregister_member ----------------------------------------------------


def test_deregister_member__active_member_returns_true():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    member = _register_member(sid, name="retiring")
    assert broker.deregister_member(member["member_id"]) is True
    names = {a["name"] for a in broker.list_roster(sid)}
    assert names == {"Director"}
    # The deregistered member still belongs to the fleet (verify_member_fleet).
    assert broker.verify_member_fleet(member["member_id"], sid) is True


@pytest.mark.parametrize(
    ("scenario", "expected"),
    [("already_deregistered", False), ("nonexistent_member", False)],
)
def test_deregister_member__idempotent_and_missing(scenario, expected):
    fleet = _create_fleet()
    if scenario == "already_deregistered":
        member = _register_member(fleet["fleet_id"], name="x")
        broker.deregister_member(member["member_id"])
        assert broker.deregister_member(member["member_id"]) is expected
    else:
        assert broker.deregister_member(999999) is expected


def test_deregister_member__deletes_placement():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    member = _register_member(sid, name="member", placement=_member_placement(None))
    broker.deregister_member(member["member_id"])
    assert broker.update_placement_pane_id(member["member_id"], "%99") is None


# --- update_placement_pane_id -------------------------------------------


def test_update_placement_pane_id__updates_and_persists():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    member = _register_member(sid, name="member", placement=_member_placement(None))
    result = broker.update_placement_pane_id(member["member_id"], "%42")
    assert result["mux_pane_id"] == "%42"
    fetched = broker.get_member(member["member_id"], sid)
    assert fetched["placement"]["mux_pane_id"] == "%42"


@pytest.mark.parametrize("scenario", ["no_placement", "nonexistent_member"])
def test_update_placement_pane_id__returns_none_for_missing(scenario):
    fleet = _create_fleet()
    if scenario == "no_placement":
        member = _register_member(fleet["fleet_id"], name="no-placement")
        assert broker.update_placement_pane_id(member["member_id"], "%99") is None
    else:
        assert broker.update_placement_pane_id(999999, "%1") is None


# --- list_members --------------------------------------------------------


def test_list_members__returns_members_with_placement_info():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    _register_member(sid, name="member-1", placement=_member_placement(None))
    _register_member(sid, name="member-2", placement=_member_placement(None))

    result = broker.list_members(sid)
    assert len(result) == 3
    assert {m["name"] for m in result} == {"Director", "member-1", "member-2"}
    member = next(m for m in result if m["name"] == "member-1")
    assert member["placement"]["mux_session"] == "main"
    assert member["kind"] == "member"


def test_list_members__includes_root_director_and_bootstrap_case():
    """``list_members(fleet_id)`` returns every active registry entry of the
    fleet — the root Director included."""
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    did = fleet["director"]["member_id"]
    _register_member(sid, name="member-1", placement=_member_placement(None))
    _register_member(sid, name="member-2", placement=_member_placement(None))

    result = broker.list_members(sid)
    assert {m["name"] for m in result} == {"Director", "member-1", "member-2"}
    assert did in {m["member_id"] for m in result}

    # Bootstrap-only fleet → just the root Director.
    bare = _create_fleet()
    assert [m["member_id"] for m in broker.list_members(bare["fleet_id"])] == [
        bare["director"]["member_id"]
    ]
