"""Tests for ``broker.list_members_with_activity`` (flat single-Director model)."""

import click
import pytest

from cafleet import broker
from cafleet.multiplexer import MultiplexerContext as DirectorContext


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


def _bootstrap_fleet():
    info = broker.create_fleet(
        label="activity-test",
        director_context=DirectorContext(session="main", window_id="@3", pane_id="%0"),
        coding_agent="claude",
    )
    return info["fleet_id"], info["director"]["agent_id"]


def _register_member(fleet_id, director_id, name, pane):
    placement = {
        "director_agent_id": director_id,
        "tmux_session": "main",
        "tmux_window_id": "@3",
        "tmux_pane_id": pane,
        "coding_agent": "claude",
    }
    agent = broker.register_agent(
        fleet_id=fleet_id,
        name=name,
        description=f"member {name}",
        placement=placement,
    )
    return agent["agent_id"]


def _setup_three_member_team():
    sid, director_id = _bootstrap_fleet()
    a = _register_member(sid, director_id, "alice", "%10")
    b = _register_member(sid, director_id, "bob", "%11")
    c = _register_member(sid, director_id, "carol", "%12")
    return sid, director_id, a, b, c


def test_list_members_with_activity__shape_identity_placement_and_activity_keys():
    sid, director_id, a, b, c = _setup_three_member_team()
    rows = broker.list_members_with_activity(sid)
    assert len(rows) == 3
    assert {row["agent_id"] for row in rows} == {a, b, c}

    alice = next(r for r in rows if r["agent_id"] == a)
    # Identity + placement match list_members superset contract.
    assert alice["name"] == "alice"
    assert alice["status"] == "active"
    # In the flat model every member's placement director is the fleet root.
    assert alice["placement"]["director_agent_id"] == director_id
    assert alice["placement"]["tmux_pane_id"] == "%10"
    # Activity keys present even with no tasks yet.
    for key in ("last_sent", "last_recv", "last_ack", "idle"):
        assert key in alice
        assert alice[key] is None


def test_list_members_with_activity__last_sent_and_last_recv_track_most_recent_timestamp():
    sid, _director_id, a, b, _c = _setup_three_member_team()
    broker.send_message(sid, a, b, "first")
    second = broker.send_message(sid, a, b, "second")
    rows = broker.list_members_with_activity(sid)
    alice = next(r for r in rows if r["agent_id"] == a)
    bob = next(r for r in rows if r["agent_id"] == b)
    assert alice["last_sent"] == second["task"]["status_timestamp"]
    assert bob["last_recv"] == second["task"]["status_timestamp"]
    # last_ack stays None until recipient acks.
    assert bob["last_ack"] is None


def test_list_members_with_activity__last_ack_tracks_real_acks_only():
    sid, _director_id, a, b, _c = _setup_three_member_team()
    sent = broker.send_message(sid, b, a, "ping")
    acked = broker.ack_task(a, sent["task"]["task_id"])
    # Broadcast summary (status_state=completed) must NOT pollute last_ack.
    broker.broadcast_message(sid, a, "team-wide note")
    rows = broker.list_members_with_activity(sid)
    alice = next(r for r in rows if r["agent_id"] == a)
    assert alice["last_ack"] == acked["task"]["status_timestamp"]


@pytest.mark.parametrize("column", ["last_recv", "last_ack"])
def test_list_members_with_activity__broadcast_summary_filtered_from_proxy_columns(
    column,
):
    sid, _director_id, a, _b, _c = _setup_three_member_team()
    # Alice broadcasts; her own broadcast_summary lands in her context with
    # status_state='completed'. Neither last_recv nor last_ack should register it.
    broker.broadcast_message(sid, a, "team-wide note")
    rows = broker.list_members_with_activity(sid)
    alice = next(r for r in rows if r["agent_id"] == a)
    assert alice[column] is None


def test_list_members_with_activity__idle_transitions_from_none_after_activity():
    sid, _director_id, a, b, _c = _setup_three_member_team()
    rows_pre = broker.list_members_with_activity(sid)
    alice_pre = next(r for r in rows_pre if r["agent_id"] == a)
    assert alice_pre["idle"] is None

    broker.send_message(sid, a, b, "hello")
    rows_post = broker.list_members_with_activity(sid)
    alice_post = next(r for r in rows_post if r["agent_id"] == a)
    assert alice_post["idle"] is not None


def test_list_members_with_activity__lists_root_members_excludes_root_and_deregistered():
    sid, director_id, a, b, c = _setup_three_member_team()
    broker.deregister_agent(c)

    rows = broker.list_members_with_activity(sid)
    agent_ids = {row["agent_id"] for row in rows}
    # Flat model: every member under the root is listed. The root Director's
    # own placement row (director_agent_id IS NULL) is excluded, and the
    # deregistered carol is gone.
    assert agent_ids == {a, b}
    assert director_id not in agent_ids

    # Empty fleet (bootstrap-only) → no members.
    sid2, _director_id2 = _bootstrap_fleet()
    assert broker.list_members_with_activity(sid2) == []

    # Unknown fleet-id returns [].
    assert broker.list_members_with_activity(999999) == []


def test_register_agent__member_under_non_root_director_is_rejected():
    """Nested teams are forbidden: a member whose placement names a non-root
    director (another member) is rejected by ``register_agent``."""
    sid, _director_id, a, _b, _c = _setup_three_member_team()
    placement = {
        "director_agent_id": a,  # a member — not the fleet root Director
        "tmux_session": "main",
        "tmux_window_id": "@3",
        "tmux_pane_id": "%20",
        "coding_agent": "claude",
    }
    with pytest.raises(click.UsageError):
        broker.register_agent(
            fleet_id=sid,
            name="nested-outsider",
            description="must be rejected",
            placement=placement,
        )


def test_register_agent__member_under_root_succeeds_and_is_listed():
    sid, director_id = _bootstrap_fleet()
    member_id = _register_member(sid, director_id, "rooted", "%30")
    rows = broker.list_members(sid)
    assert member_id in {r["agent_id"] for r in rows}
