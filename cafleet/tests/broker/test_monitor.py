"""Broker DB-layer tests for the monitor schedule + runtime functions.

Covers enrollment-on-registration, cleanup-on-teardown, per-agent config
edits, the per-tick scan (``list_monitor_targets``), ping recording, and the
single-instance runtime claim/heartbeat/clear with the ownership-checked
split-brain guard.
"""

import os
from datetime import UTC, datetime, timedelta

import click
import pytest

from cafleet import broker
from cafleet.db.models import MonitorConfig, MonitorRuntime, Task
from tests.broker._helpers import _create_fleet, _register_agent


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


def _register_member(fleet: dict, name: str = "member", pane_id: str = "%5") -> dict:
    """Register a pane-bound member under the fleet's root Director."""
    return _register_agent(
        fleet["fleet_id"],
        name=name,
        placement={
            "director_agent_id": fleet["director"]["agent_id"],
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": pane_id,
            "coding_agent": "claude",
        },
    )


def _iso_now() -> str:
    return datetime.now(UTC).isoformat()


# --- enrollment on registration -------------------------------------------


def test_enroll_on_create__director_enrolled_administrator_not():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]

    director_cfg = broker.get_monitor_config(sid, fleet["director"]["agent_id"])
    assert director_cfg is not None
    assert director_cfg["interval_seconds"] == 60
    assert director_cfg["enabled"] is True

    assert broker.get_monitor_config(sid, fleet["administrator_agent_id"]) is None


def test_enroll_on_register__member_with_placement_enrolled():
    fleet = _create_fleet()
    member = _register_member(fleet, name="alice")

    cfg = broker.get_monitor_config(fleet["fleet_id"], member["agent_id"])
    assert cfg is not None
    assert cfg["interval_seconds"] == 60
    assert cfg["enabled"] is True
    assert cfg["last_ping_at"] is None


def test_enroll_on_register__card_only_agent_not_enrolled():
    fleet = _create_fleet()
    card_only = _register_agent(fleet["fleet_id"], name="card-only")  # no placement

    assert broker.get_monitor_config(fleet["fleet_id"], card_only["agent_id"]) is None


# --- cleanup on teardown ---------------------------------------------------


def test_cleanup_on_deregister__config_removed():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    member = _register_member(fleet, name="bob")

    assert broker.get_monitor_config(sid, member["agent_id"]) is not None
    broker.deregister_agent(member["agent_id"])
    assert broker.get_monitor_config(sid, member["agent_id"]) is None


def test_cleanup_on_fleet_delete__config_and_runtime_removed(broker_session):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    member = _register_member(fleet, name="carol")
    member_id = member["agent_id"]
    broker.claim_monitor_runtime(sid, os.getpid(), 5, _iso_now())

    broker.delete_fleet(sid)

    with broker_session() as s:
        configs = (
            s.query(MonitorConfig)
            .filter(MonitorConfig.agent_id.in_([director_id, member_id]))
            .all()
        )
        runtime = s.get(MonitorRuntime, sid)
    assert configs == []
    assert runtime is None


# --- update_monitor_config -------------------------------------------------


def test_update_monitor_config__interval_and_enabled_persist():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    aid = _register_member(fleet, name="dave")["agent_id"]

    updated = broker.update_monitor_config(sid, aid, interval_seconds=30)
    assert updated["interval_seconds"] == 30

    disabled = broker.update_monitor_config(sid, aid, enabled=False)
    assert disabled["enabled"] is False
    assert isinstance(disabled["enabled"], bool)

    enabled = broker.update_monitor_config(sid, aid, enabled=True)
    assert enabled["enabled"] is True

    # a partial update must not clobber the other field
    persisted = broker.get_monitor_config(sid, aid)
    assert persisted["interval_seconds"] == 30
    assert persisted["enabled"] is True


def test_update_monitor_config__unknown_agent_raises():
    fleet = _create_fleet()
    with pytest.raises(click.ClickException):
        broker.update_monitor_config(fleet["fleet_id"], 999999, interval_seconds=30)


def test_update_monitor_config__not_enrolled_administrator_raises():
    fleet = _create_fleet()
    with pytest.raises(click.ClickException):
        broker.update_monitor_config(
            fleet["fleet_id"], fleet["administrator_agent_id"], enabled=False
        )


def test_update_monitor_config__cross_fleet_raises():
    fleet_a = _create_fleet()
    fleet_b = _create_fleet()
    member = _register_member(fleet_a, name="erin")
    with pytest.raises(click.ClickException):
        broker.update_monitor_config(
            fleet_b["fleet_id"], member["agent_id"], interval_seconds=30
        )


# --- record_ping -----------------------------------------------------------


def test_record_ping__sets_last_ping_at():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    aid = _register_member(fleet, name="frank")["agent_id"]

    assert broker.get_monitor_config(sid, aid)["last_ping_at"] is None
    when = _iso_now()
    broker.record_ping(aid, when)
    assert broker.get_monitor_config(sid, aid)["last_ping_at"] == when


# --- list_monitor_configs --------------------------------------------------


def test_list_monitor_configs__enrolled_only_and_bool_enabled():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    member = _register_member(fleet, name="gina")
    _register_agent(sid, name="card-only")  # not enrolled

    configs = broker.list_monitor_configs(sid)
    enrolled_ids = {c["agent_id"] for c in configs}
    assert enrolled_ids == {fleet["director"]["agent_id"], member["agent_id"]}
    for c in configs:
        assert isinstance(c["enabled"], bool)


# --- list_monitor_targets (per-tick scan) ----------------------------------


def test_list_monitor_targets__director_and_member_shape():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    member = _register_member(fleet, name="hank", pane_id="%7")
    member_id = member["agent_id"]
    _register_agent(sid, name="card-only")  # not enrolled → excluded

    targets = {t["agent_id"]: t for t in broker.list_monitor_targets(sid)}
    assert set(targets) == {director_id, member_id}

    d = targets[director_id]
    assert d["is_director"] is True
    assert d["name"] == "Director"
    assert d["pane_id"] == "%0"
    assert d["interval_seconds"] == 60
    assert d["last_ping_at"] is None
    assert isinstance(d["enabled"], bool)
    assert d["enabled"] is True
    assert d["pending_count"] == 0

    m = targets[member_id]
    assert m["is_director"] is False
    assert m["name"] == "hank"
    assert m["pane_id"] == "%7"
    assert m["pending_count"] == 0


def test_list_monitor_targets__pending_count_input_required_only():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    member_id = _register_member(fleet, name="ivy", pane_id="%9")["agent_id"]

    first = broker.send_message(sid, director_id, member_id, "msg1")
    broker.send_message(sid, director_id, member_id, "msg2")
    broker.send_message(sid, director_id, member_id, "msg3")

    targets = {t["agent_id"]: t for t in broker.list_monitor_targets(sid)}
    assert targets[member_id]["pending_count"] == 3
    assert targets[director_id]["pending_count"] == 0

    broker.ack_task(member_id, first["task"]["task_id"])
    targets = {t["agent_id"]: t for t in broker.list_monitor_targets(sid)}
    assert targets[member_id]["pending_count"] == 2


def test_list_monitor_targets__pending_count_excludes_broadcast_summary(broker_session):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    member_id = _register_member(fleet, name="jack", pane_id="%11")["agent_id"]

    broker.send_message(sid, director_id, member_id, "real pending")

    now = _iso_now()
    with broker_session() as s:
        s.add(
            Task(
                context_id=member_id,
                from_agent_id=director_id,
                to_agent_id=0,
                type="broadcast_summary",
                created_at=now,
                status_state="input_required",
                status_timestamp=now,
                origin_task_id=None,
                text="summary",
            )
        )
        s.commit()

    targets = {t["agent_id"]: t for t in broker.list_monitor_targets(sid)}
    assert targets[member_id]["pending_count"] == 1


def test_list_monitor_targets__excludes_deregistered_member():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    member = _register_member(fleet, name="kim")
    broker.deregister_agent(member["agent_id"])

    target_ids = {t["agent_id"] for t in broker.list_monitor_targets(sid)}
    assert member["agent_id"] not in target_ids


# --- runtime claim / heartbeat / clear / read ------------------------------


def test_claim_monitor_runtime__fresh_claim_succeeds():
    sid = _create_fleet()["fleet_id"]
    now = _iso_now()

    assert broker.claim_monitor_runtime(sid, os.getpid(), 5, now) is True
    row = broker.read_monitor_runtime(sid)
    assert row is not None
    assert row["pid"] == os.getpid()
    assert row["tick_seconds"] == 5
    assert row["started_at"] == now
    assert row["last_tick_at"] == now


def test_claim_monitor_runtime__refuses_live_row():
    sid = _create_fleet()["fleet_id"]

    assert broker.claim_monitor_runtime(sid, os.getpid(), 5, _iso_now()) is True
    # the slot is held by this live process with a fresh heartbeat → refused
    assert broker.claim_monitor_runtime(sid, os.getpid() + 1, 5, _iso_now()) is False


def test_claim_monitor_runtime__reclaims_stale_row():
    sid = _create_fleet()["fleet_id"]
    stale = (datetime.now(UTC) - timedelta(hours=1)).isoformat()

    assert broker.claim_monitor_runtime(sid, os.getpid(), 5, stale) is True
    fresh = _iso_now()
    assert broker.claim_monitor_runtime(sid, os.getpid(), 5, fresh) is True
    assert broker.read_monitor_runtime(sid)["last_tick_at"] == fresh


def test_heartbeat_monitor_runtime__owner_updates():
    sid = _create_fleet()["fleet_id"]
    pid = os.getpid()
    broker.claim_monitor_runtime(sid, pid, 5, _iso_now())

    later = _iso_now()
    assert broker.heartbeat_monitor_runtime(sid, pid, later) is True
    assert broker.read_monitor_runtime(sid)["last_tick_at"] == later


def test_heartbeat_monitor_runtime__non_owner_returns_false():
    sid = _create_fleet()["fleet_id"]
    pid = os.getpid()
    broker.claim_monitor_runtime(sid, pid, 5, _iso_now())

    assert broker.heartbeat_monitor_runtime(sid, pid + 1, _iso_now()) is False


def test_heartbeat_monitor_runtime__false_after_reclaim_split_brain_guard():
    sid = _create_fleet()["fleet_id"]
    displaced, winner = 100001, 100002
    stale = (datetime.now(UTC) - timedelta(hours=1)).isoformat()

    assert broker.claim_monitor_runtime(sid, displaced, 5, stale) is True
    assert broker.claim_monitor_runtime(sid, winner, 5, _iso_now()) is True

    # the displaced owner's heartbeat matches zero rows → self-terminates
    assert broker.heartbeat_monitor_runtime(sid, displaced, _iso_now()) is False
    # the winner keeps the slot
    assert broker.heartbeat_monitor_runtime(sid, winner, _iso_now()) is True


def test_clear_monitor_runtime__owner_clears():
    sid = _create_fleet()["fleet_id"]
    pid = os.getpid()
    broker.claim_monitor_runtime(sid, pid, 5, _iso_now())

    broker.clear_monitor_runtime(sid, pid)
    row = broker.read_monitor_runtime(sid)
    assert row is not None
    assert row["pid"] is None
    assert row["last_tick_at"] is None
    # a cleanly-stopped monitor leaves no residual start time either
    assert row["started_at"] is None


def test_clear_monitor_runtime__non_owner_noop():
    sid = _create_fleet()["fleet_id"]
    pid = os.getpid()
    now = _iso_now()
    broker.claim_monitor_runtime(sid, pid, 5, now)

    broker.clear_monitor_runtime(sid, pid + 1)  # not the owner → no-op
    row = broker.read_monitor_runtime(sid)
    assert row["pid"] == pid
    assert row["last_tick_at"] == now


def test_read_monitor_runtime__none_when_absent():
    sid = _create_fleet()["fleet_id"]
    assert broker.read_monitor_runtime(sid) is None


def test_read_monitor_runtime__round_trips_claim():
    sid = _create_fleet()["fleet_id"]
    pid = os.getpid()
    now = _iso_now()
    broker.claim_monitor_runtime(sid, pid, 7, now)

    row = broker.read_monitor_runtime(sid)
    assert row["pid"] == pid
    assert row["tick_seconds"] == 7
    assert row["started_at"] == now
    assert row["last_tick_at"] == now
