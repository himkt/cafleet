"""Tests for the activity aggregation of ``broker.list_members``."""

import pytest

from cafleet import broker
from cafleet.multiplexer import MultiplexerContext as DirectorContext


def _bootstrap_fleet():
    info = broker.create_fleet(
        name="activity-test",
        director_context=DirectorContext(session="main", window_id="@3", pane_id="%0"),
        coding_agent="claude",
        backend="tmux",
    )
    return info["fleet_id"], info["director"]["member_id"]


def _register_member(fleet_id, name, pane):
    placement = {
        "backend": "tmux",
        "mux_session": "main",
        "mux_window_id": "@3",
        "mux_pane_id": pane,
        "coding_agent": "claude",
    }
    member = broker.register_member(
        fleet_id=fleet_id,
        name=name,
        description=f"member {name}",
        placement=placement,
    )
    return member["member_id"]


def _setup_three_member_team():
    sid, director_id = _bootstrap_fleet()
    a = _register_member(sid, "alice", "%10")
    b = _register_member(sid, "bob", "%11")
    c = _register_member(sid, "carol", "%12")
    return sid, director_id, a, b, c


def test_list_members__shape_identity_placement_and_activity_keys():
    sid, director_id, a, b, c = _setup_three_member_team()
    rows = broker.list_members(sid)
    assert len(rows) == 4
    assert {row["member_id"] for row in rows} == {director_id, a, b, c}

    alice = next(r for r in rows if r["member_id"] == a)
    assert alice["name"] == "alice"
    assert alice["kind"] == "member"
    assert alice["placement"]["mux_pane_id"] == "%10"
    # Activity keys present even with no messages yet.
    for key in ("last_sent", "last_recv", "last_ack", "idle"):
        assert key in alice
        assert alice[key] is None


def test_list_members__last_sent_and_last_recv_track_most_recent_timestamp():
    sid, _director_id, a, b, _c = _setup_three_member_team()
    broker.send_message(sid, a, b, "first")
    second = broker.send_message(sid, a, b, "second")
    rows = broker.list_members(sid)
    alice = next(r for r in rows if r["member_id"] == a)
    bob = next(r for r in rows if r["member_id"] == b)
    assert alice["last_sent"] == second["message"]["status_timestamp"]
    assert bob["last_recv"] == second["message"]["status_timestamp"]
    # last_ack stays None until recipient acks.
    assert bob["last_ack"] is None


def test_list_members__last_ack_tracks_real_acks_only():
    sid, _director_id, a, b, _c = _setup_three_member_team()
    sent = broker.send_message(sid, b, a, "ping")
    acked = broker.ack_message(a, sent["message"]["message_id"])
    # Broadcast summary (status_state=completed) must NOT pollute last_ack.
    broker.broadcast_message(sid, a, "team-wide note")
    rows = broker.list_members(sid)
    alice = next(r for r in rows if r["member_id"] == a)
    assert alice["last_ack"] == acked["message"]["status_timestamp"]


@pytest.mark.parametrize("column", ["last_recv", "last_ack"])
def test_list_members__broadcast_summary_filtered_from_proxy_columns(
    column,
):
    sid, _director_id, a, _b, _c = _setup_three_member_team()
    # Alice broadcasts; her own broadcast_summary lands in her context with
    # status_state='completed'. Neither last_recv nor last_ack should register it.
    broker.broadcast_message(sid, a, "team-wide note")
    rows = broker.list_members(sid)
    alice = next(r for r in rows if r["member_id"] == a)
    assert alice[column] is None


def test_list_members__idle_transitions_from_none_after_activity():
    sid, _director_id, a, b, _c = _setup_three_member_team()
    rows_pre = broker.list_members(sid)
    alice_pre = next(r for r in rows_pre if r["member_id"] == a)
    assert alice_pre["idle"] is None

    broker.send_message(sid, a, b, "hello")
    rows_post = broker.list_members(sid)
    alice_post = next(r for r in rows_post if r["member_id"] == a)
    assert alice_post["idle"] is not None


def test_list_members__includes_root_director_excludes_deregistered():
    sid, director_id, a, b, c = _setup_three_member_team()
    broker.deregister_member(c)

    rows = broker.list_members(sid)
    member_ids = {row["member_id"] for row in rows}
    # Every active registry entry is listed — the root Director included; the
    # deregistered carol is gone.
    assert member_ids == {director_id, a, b}

    # Bootstrap-only fleet → just the root Director.
    sid2, director_id2 = _bootstrap_fleet()
    assert [r["member_id"] for r in broker.list_members(sid2)] == [director_id2]

    # Unknown fleet-id returns [].
    assert broker.list_members(999999) == []


def test_register_member__placed_member_is_listed():
    sid, _director_id = _bootstrap_fleet()
    member_id = _register_member(sid, "rooted", "%30")
    rows = broker.list_members(sid)
    assert member_id in {r["member_id"] for r in rows}
