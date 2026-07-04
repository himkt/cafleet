"""Broker DB-layer tests for the monitor schedule + runtime functions.

Covers per-member enrollment-on-registration (the root Director @180 at
``create_fleet``, every ordinary member @720 at ``register_agent``; the
monitoring member and the Administrator stay unenrolled), the
``monitoring-member`` kind marker, the ``find_monitoring_member`` watcher lookup,
cleanup-on-teardown, per-agent config edits, the per-tick scan
(``list_monitor_targets`` over the watched Director + members, with no
``is_monitoring_member`` field), ping recording, the single-instance runtime
claim/heartbeat/clear with the ownership-checked split-brain guard, and the
Alembic data migrations that prune ``monitor_config`` rows (``0003`` prunes
non-Director rows; ``0004`` then prunes the root-Director rows; ``0005`` —
which inverts to per-member intervals — is covered by
``test_alembic_smoke.py``).
"""

import importlib.resources
import json
import os
from datetime import UTC, datetime, timedelta

import click
import pytest
from alembic import command
from alembic.config import Config
from sqlalchemy import create_engine
from sqlalchemy.orm import Session

from cafleet import broker
from cafleet.broker import _shared
from cafleet.db.models import Agent, Fleet, MonitorConfig, MonitorRuntime, Task
from tests.broker._helpers import _create_fleet, _register_agent


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


def _placement(fleet: dict, pane_id: str = "%5") -> dict:
    return {
        "director_agent_id": fleet["director"]["agent_id"],
        "tmux_session": "main",
        "tmux_window_id": "@3",
        "tmux_pane_id": pane_id,
        "coding_agent": "claude",
    }


def _register_ordinary_member(
    fleet: dict, name: str = "member", pane_id: str = "%5"
) -> dict:
    """A pane-bound ordinary member — enrolled in monitoring @720 (this design)."""
    return _register_agent(
        fleet["fleet_id"], name=name, placement=_placement(fleet, pane_id)
    )


def _register_monitoring_member(
    fleet: dict, name: str = "watcher", pane_id: str = "%5"
) -> dict:
    """The dedicated monitoring member — the unenrolled watcher, located by kind."""
    return broker.register_agent(
        fleet_id=fleet["fleet_id"],
        name=name,
        description="monitoring member",
        placement=_placement(fleet, pane_id),
        kind="monitoring-member",
    )


def _iso_now() -> str:
    return datetime.now(UTC).isoformat()


# --- enrollment on registration -------------------------------------------


def test_enroll_on_create__director_enrolled_180_administrator_not():
    # this design: create_fleet enrolls the root Director @180; the Administrator
    # has no placement and stays unenrolled.
    fleet = _create_fleet()
    sid = fleet["fleet_id"]

    director_cfg = broker.get_monitor_config(sid, fleet["director"]["agent_id"])
    assert director_cfg is not None
    assert director_cfg["interval_seconds"] == 180
    assert director_cfg["enabled"] is True
    assert director_cfg["last_ping_at"] is None

    assert broker.get_monitor_config(sid, fleet["administrator_agent_id"]) is None


def test_enroll_on_register__ordinary_member_enrolled_720():
    # this design: a pane-bound ordinary member is enrolled @720 at registration.
    fleet = _create_fleet()
    member = _register_ordinary_member(fleet, name="alice")

    cfg = broker.get_monitor_config(fleet["fleet_id"], member["agent_id"])
    assert cfg is not None
    assert cfg["interval_seconds"] == 720
    assert cfg["enabled"] is True
    assert cfg["last_ping_at"] is None


def test_enroll_on_register__monitoring_member_not_enrolled():
    # this design: the monitoring member is the unenrolled watcher (located by
    # kind), so register_agent does NOT write a monitor_config row for it.
    fleet = _create_fleet()
    watcher = _register_monitoring_member(fleet, name="watcher")

    assert broker.get_monitor_config(fleet["fleet_id"], watcher["agent_id"]) is None


def test_enroll_on_register__card_only_agent_not_enrolled():
    fleet = _create_fleet()
    card_only = _register_agent(fleet["fleet_id"], name="card-only")  # no placement

    assert broker.get_monitor_config(fleet["fleet_id"], card_only["agent_id"]) is None


# --- monitoring-member kind marker (§3) ------------------------------------


def test_monitoring_member_kind_constant():
    assert _shared.MONITORING_MEMBER_KIND == "monitoring-member"


@pytest.mark.parametrize(
    ("card", "expected"),
    [
        (json.dumps({"cafleet": {"kind": "monitoring-member"}}), True),
        (json.dumps({"cafleet": {"kind": "builtin-administrator"}}), False),
        (json.dumps({"name": "x", "skills": []}), False),
        (None, False),
        ("", False),
        ("not-json", False),
    ],
)
def test_is_monitoring_member__kind_detection(card, expected):
    assert _shared.is_monitoring_member(card) is expected


def test_register_monitoring_member__writes_kind_marker(broker_session):
    fleet = _create_fleet()
    watcher = _register_monitoring_member(fleet, name="watcher")

    with broker_session() as s:
        card = s.get(Agent, watcher["agent_id"]).agent_card_json
    assert _shared.is_monitoring_member(card) is True


def test_register_second_monitoring_member__rejected():
    fleet = _create_fleet()
    _register_monitoring_member(fleet, name="watcher-1", pane_id="%5")
    with pytest.raises(click.ClickException, match="monitoring member"):
        _register_monitoring_member(fleet, name="watcher-2", pane_id="%6")


def test_register_paneless_monitoring_member__rejected():
    # a monitoring member must own a pane (placement) to run the monitor loop
    # and receive wake nudges; registering one with placement=None is rejected.
    fleet = _create_fleet()
    with pytest.raises(click.ClickException, match="pane-bound"):
        broker.register_agent(
            fleet_id=fleet["fleet_id"],
            name="paneless-watcher",
            description="monitoring member without a pane",
            placement=None,
            kind="monitoring-member",
        )


# --- find_monitoring_member (watcher lookup, §3) ---------------------------


def test_find_monitoring_member__returns_pane_when_present():
    fleet = _create_fleet()
    watcher = _register_monitoring_member(fleet, name="watcher", pane_id="%7")

    found = broker.find_monitoring_member(fleet["fleet_id"])
    assert found is not None
    assert found["agent_id"] == watcher["agent_id"]
    assert found["name"] == "watcher"
    assert found["pane_id"] == "%7"


def test_find_monitoring_member__none_when_no_monitoring_member():
    fleet = _create_fleet()
    _register_ordinary_member(fleet, name="ordinary")  # not a monitoring member

    assert broker.find_monitoring_member(fleet["fleet_id"]) is None


def test_find_monitoring_member__none_after_deregister():
    fleet = _create_fleet()
    watcher = _register_monitoring_member(fleet, name="watcher")
    broker.deregister_agent(watcher["agent_id"])

    assert broker.find_monitoring_member(fleet["fleet_id"]) is None


def test_find_monitoring_member__none_when_pane_pending():
    # a monitoring member whose placement is still pending (tmux_pane_id=None) has
    # no wakeable pane, so find_monitoring_member treats it as absent.
    fleet = _create_fleet()
    broker.register_agent(
        fleet_id=fleet["fleet_id"],
        name="pending-watcher",
        description="monitoring member with a pending pane",
        placement={
            "director_agent_id": fleet["director"]["agent_id"],
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": None,
            "coding_agent": "claude",
        },
        kind="monitoring-member",
    )

    assert broker.find_monitoring_member(fleet["fleet_id"]) is None


def test_find_monitoring_member__scoped_to_fleet():
    fleet_a = _create_fleet()
    fleet_b = _create_fleet()
    _register_monitoring_member(fleet_b, name="watcher-b", pane_id="%7")

    # fleet_a has no monitoring member of its own; fleet_b's is not leaked to it
    assert broker.find_monitoring_member(fleet_a["fleet_id"]) is None
    assert broker.find_monitoring_member(fleet_b["fleet_id"])["name"] == "watcher-b"


# --- cleanup on teardown ---------------------------------------------------


def test_cleanup_on_deregister__config_removed():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    member = _register_ordinary_member(fleet, name="bob")

    assert broker.get_monitor_config(sid, member["agent_id"]) is not None
    broker.deregister_agent(member["agent_id"])
    assert broker.get_monitor_config(sid, member["agent_id"]) is None


def test_cleanup_on_fleet_delete__config_and_runtime_removed(broker_session):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    member_id = _register_ordinary_member(fleet, name="carol")["agent_id"]
    broker.claim_monitor_runtime(sid, os.getpid(), 5, _iso_now())

    # both the Director (@180) and the member (@720) hold monitor_config rows
    assert broker.get_monitor_config(sid, director_id) is not None
    assert broker.get_monitor_config(sid, member_id) is not None

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
    # an ordinary member is enrolled @720 (this design), so it exercises the
    # config-edit mechanics
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    aid = _register_ordinary_member(fleet, name="member")["agent_id"]

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
    member = _register_ordinary_member(fleet_a, name="erin")
    with pytest.raises(click.ClickException):
        broker.update_monitor_config(
            fleet_b["fleet_id"], member["agent_id"], interval_seconds=30
        )


# --- record_ping -----------------------------------------------------------


def test_record_ping__sets_last_ping_at():
    # an ordinary member is enrolled @720 (this design)
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    aid = _register_ordinary_member(fleet, name="pinger")["agent_id"]

    assert broker.get_monitor_config(sid, aid)["last_ping_at"] is None
    when = _iso_now()
    broker.record_ping(aid, when)
    assert broker.get_monitor_config(sid, aid)["last_ping_at"] == when


# --- list_monitor_configs --------------------------------------------------


def test_list_monitor_configs__enrolled_only_and_bool_enabled():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    member = _register_ordinary_member(fleet, name="ordinary", pane_id="%8")
    _register_monitoring_member(fleet, name="gina", pane_id="%5")  # NOT enrolled

    configs = broker.list_monitor_configs(sid)
    enrolled_ids = {c["agent_id"] for c in configs}
    # this design: the Director (@180) + ordinary members (@720) are enrolled;
    # the monitoring member is the unenrolled watcher.
    assert enrolled_ids == {director_id, member["agent_id"]}
    for c in configs:
        assert isinstance(c["enabled"], bool)


# --- list_monitor_targets (per-tick scan) ----------------------------------


def test_list_monitor_targets__director_and_members_shape():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    member = _register_ordinary_member(fleet, name="hank", pane_id="%8")
    member_id = member["agent_id"]
    watcher = _register_monitoring_member(fleet, name="watcher", pane_id="%7")

    targets = {t["agent_id"]: t for t in broker.list_monitor_targets(sid)}
    # this design: the watched set is the Director + ordinary members; the
    # monitoring member is the unenrolled watcher and is NOT a target.
    assert set(targets) == {director_id, member_id}
    assert watcher["agent_id"] not in targets

    # the dead is_monitoring_member field is removed from every row
    for t in targets.values():
        assert "is_monitoring_member" not in t

    d = targets[director_id]
    assert d["is_director"] is True
    assert d["interval_seconds"] == 180
    assert d["enabled"] is True

    m = targets[member_id]
    assert m["is_director"] is False
    assert m["name"] == "hank"
    assert m["pane_id"] == "%8"
    assert m["interval_seconds"] == 720
    assert m["last_ping_at"] is None
    assert isinstance(m["enabled"], bool)
    assert m["enabled"] is True
    assert m["pending_count"] == 0


def test_list_monitor_targets__pending_count_input_required_only():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    member_id = _register_ordinary_member(fleet, name="ivy", pane_id="%9")["agent_id"]

    first = broker.send_message(sid, director_id, member_id, "msg1")
    broker.send_message(sid, director_id, member_id, "msg2")
    broker.send_message(sid, director_id, member_id, "msg3")

    targets = {t["agent_id"]: t for t in broker.list_monitor_targets(sid)}
    assert targets[member_id]["pending_count"] == 3
    # the Director is a watched target too, but has no pending input here
    assert targets[director_id]["pending_count"] == 0

    broker.ack_task(member_id, first["task"]["task_id"])
    targets = {t["agent_id"]: t for t in broker.list_monitor_targets(sid)}
    assert targets[member_id]["pending_count"] == 2


def test_list_monitor_targets__pending_count_excludes_broadcast_summary(broker_session):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["agent_id"]
    member_id = _register_ordinary_member(fleet, name="jack", pane_id="%11")["agent_id"]

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
    member = _register_ordinary_member(fleet, name="kim")

    target_ids = {t["agent_id"] for t in broker.list_monitor_targets(sid)}
    assert member["agent_id"] in target_ids

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


# --- Alembic data migrations: prune monitor_config rows --------------------

_SEED_TS = "2026-06-14T00:00:00+00:00"


def _seed_legacy_monitor_config(url: str) -> None:
    """Seed a fleet + Director (10) + ordinary member (20), each with a
    ``monitor_config`` row.

    FK-safe ordering (``PRAGMA foreign_keys=ON`` is global): fleet with a NULL
    ``director_agent_id`` → agents → patch the Director FK → ``monitor_config``.
    """
    engine = create_engine(url)
    try:
        with Session(engine) as s:
            s.add(Fleet(fleet_id=1, created_at=_SEED_TS))
            s.flush()
            s.add_all(
                [
                    Agent(
                        agent_id=10,
                        fleet_id=1,
                        name="Director",
                        description="director",
                        status="active",
                        registered_at=_SEED_TS,
                        agent_card_json="{}",
                    ),
                    Agent(
                        agent_id=20,
                        fleet_id=1,
                        name="legacy-member",
                        description="member",
                        status="active",
                        registered_at=_SEED_TS,
                        agent_card_json="{}",
                    ),
                ]
            )
            s.flush()
            s.get(Fleet, 1).director_agent_id = 10
            s.add_all(
                [
                    MonitorConfig(agent_id=10, interval_seconds=60, enabled=1),
                    MonitorConfig(agent_id=20, interval_seconds=60, enabled=1),
                ]
            )
            s.commit()
    finally:
        engine.dispose()


def _monitor_config_agent_ids(url: str) -> set[int]:
    engine = create_engine(url)
    try:
        with Session(engine) as s:
            return {row.agent_id for row in s.query(MonitorConfig).all()}
    finally:
        engine.dispose()


def test_prune_migration__deletes_non_director_rows_downgrade_noop(tmp_path):
    db_file = tmp_path / "cafleet.db"
    url = f"sqlite:///{db_file}"

    # ``as_file`` materializes the bundled alembic.ini (mirrors cli/db.py); hold
    # the context open across every command.* call so the file survives.
    with importlib.resources.as_file(
        importlib.resources.files("cafleet.db") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", url)

        # 0002 introduced monitor_config; the prune (0003) is not yet applied.
        command.upgrade(cfg, "0002")
        _seed_legacy_monitor_config(url)
        assert _monitor_config_agent_ids(url) == {10, 20}

        # 0003 prunes the non-Director row (20) and keeps the root-Director row
        # (10). Pin to 0003 (not head) so this exercises 0003 in isolation —
        # 0004 (which deletes row 10) is covered by the 0004 test.
        command.upgrade(cfg, "0003")
        assert _monitor_config_agent_ids(url) == {10}

        # the downgrade is a no-op — it does NOT resurrect the pruned row.
        command.downgrade(cfg, "0002")
        assert _monitor_config_agent_ids(url) == {10}


def _seed_director_and_monitoring_member(url: str) -> None:
    """Seed a fleet + Director (10) + monitoring member (30), each with a
    ``monitor_config`` row, at the 0003 boundary (before 0004 prunes Director).

    FK-safe ordering mirrors ``_seed_legacy_monitor_config``: fleet with a NULL
    ``director_agent_id`` → agents → patch the Director FK → ``monitor_config``.
    """
    engine = create_engine(url)
    try:
        with Session(engine) as s:
            s.add(Fleet(fleet_id=1, created_at=_SEED_TS))
            s.flush()
            s.add_all(
                [
                    Agent(
                        agent_id=10,
                        fleet_id=1,
                        name="Director",
                        description="director",
                        status="active",
                        registered_at=_SEED_TS,
                        agent_card_json="{}",
                    ),
                    Agent(
                        agent_id=30,
                        fleet_id=1,
                        name="watcher",
                        description="monitoring member",
                        status="active",
                        registered_at=_SEED_TS,
                        agent_card_json=json.dumps(
                            {"cafleet": {"kind": "monitoring-member"}}
                        ),
                    ),
                ]
            )
            s.flush()
            s.get(Fleet, 1).director_agent_id = 10
            s.add_all(
                [
                    MonitorConfig(agent_id=10, interval_seconds=60, enabled=1),
                    MonitorConfig(agent_id=30, interval_seconds=60, enabled=1),
                ]
            )
            s.commit()
    finally:
        engine.dispose()


def test_prune_director_migration__deletes_director_row_keeps_monitoring_member(
    tmp_path,
):
    db_file = tmp_path / "cafleet.db"
    url = f"sqlite:///{db_file}"

    with importlib.resources.as_file(
        importlib.resources.files("cafleet.db") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", url)

        # at the 0003 boundary, seed a Director (10) + monitoring member (30),
        # both enrolled (this is the post-0003 / pre-0004 world).
        command.upgrade(cfg, "0003")
        _seed_director_and_monitoring_member(url)
        assert _monitor_config_agent_ids(url) == {10, 30}

        # 0004 prunes the root-Director row (10) and keeps the monitoring
        # member's row (30). Pin to 0004 (not head) so this exercises 0004 in
        # isolation — 0005 (the new head) prunes the monitoring member and
        # backfills the Director, which the 0005 test covers.
        command.upgrade(cfg, "0004")
        assert _monitor_config_agent_ids(url) == {30}

        # the downgrade is a no-op — it does NOT resurrect the Director row.
        command.downgrade(cfg, "0003")
        assert _monitor_config_agent_ids(url) == {30}
