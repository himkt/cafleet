"""WebUI API tests for the monitor surface (§8).

Endpoint tests need a DB that the FastAPI threadpool worker shares with the
test thread. The default per-test ``broker_session`` (in-memory,
``SingletonThreadPool``) is per-thread, so sync endpoints run in a worker
thread would see an empty DB. ``StaticPool`` + ``check_same_thread=False``
shares one in-memory connection across threads — the idiomatic FastAPI +
SQLite test setup — while the global ``PRAGMA foreign_keys=ON`` listener still
applies.

The watched set is the root Director (180 s) + every ordinary member (720 s),
so each folds a non-null ``monitor`` object; the dedicated monitoring member is
the unenrolled watcher, so it folds ``null``. No API/SPA code change is
required for the edit surface — the enrollment flip alone drives the folded
field (§8).
"""

import os
from datetime import UTC, datetime, timedelta

import click
import pytest
from fastapi.testclient import TestClient
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker
from sqlalchemy.pool import StaticPool

from cafleet import broker
from cafleet.broker import _shared
from cafleet.db.models import Base
from cafleet.webui.app import create_app
from tests.broker._helpers import (
    _create_fleet,
    _member_placement,
    _register_monitoring_member,
)


@pytest.fixture
def api_db(monkeypatch):
    engine = create_engine(
        "sqlite://", connect_args={"check_same_thread": False}, poolclass=StaticPool
    )
    Base.metadata.create_all(engine)
    sm = sessionmaker(engine, expire_on_commit=False)
    monkeypatch.setattr(_shared, "get_sync_sessionmaker", lambda: sm)
    return sm


@pytest.fixture
def client(tmp_path):
    return TestClient(create_app(str(tmp_path / "nodist")))


def _register_member(fleet: dict, name: str = "member", pane_id: str = "%5") -> int:
    return broker.register_member(
        fleet_id=fleet["fleet_id"],
        name=name,
        description="m",
        placement=_member_placement(pane_id),
    )["member_id"]


def _headers(fleet_id: int) -> dict:
    return {"X-Fleet-Id": str(fleet_id)}


# --- GET /api/monitor ------------------------------------------------------


def test_get_monitor__stopped_when_no_runtime(api_db, client):
    sid = _create_fleet()["fleet_id"]
    resp = client.get("/api/monitor", headers=_headers(sid))
    assert resp.status_code == 200, resp.text
    data = resp.json()
    assert data["running"] is False
    assert data["pid"] is None


def test_get_monitor__running_after_claim(api_db, client):
    sid = _create_fleet()["fleet_id"]
    broker.claim_monitor_runtime(sid, os.getpid(), 5, datetime.now(UTC).isoformat())

    resp = client.get("/api/monitor", headers=_headers(sid))
    assert resp.status_code == 200, resp.text
    data = resp.json()
    assert data["running"] is True
    assert data["pid"] == os.getpid()
    assert data["tick_seconds"] == 5


def test_get_monitor__stale_row_reports_not_running_with_nulls(api_db, client):
    # a runtime row exists but its heartbeat is stale (beyond STALE_AFTER): not
    # live, so the process fields null out — no lingering pid/started_at leaks
    sid = _create_fleet()["fleet_id"]
    stale = (datetime.now(UTC) - timedelta(hours=1)).isoformat()
    broker.claim_monitor_runtime(sid, os.getpid(), 5, stale)

    resp = client.get("/api/monitor", headers=_headers(sid))
    assert resp.status_code == 200, resp.text
    data = resp.json()
    assert data["running"] is False
    assert data["pid"] is None
    assert data["started_at"] is None
    assert data["last_tick_at"] is None
    assert data["tick_seconds"] == 5  # the row exists (stale, not absent)


# --- GET /api/members folded monitor field ----------------------------------


def test_get_members__monitor_field_folded(api_db, client):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]
    member_id = _register_member(fleet, name="alice")
    watcher_id = _register_monitoring_member(fleet, pane_id="%7")["member_id"]

    resp = client.get("/api/members", headers=_headers(sid))
    assert resp.status_code == 200, resp.text
    members = {m["member_id"]: m for m in resp.json()["members"]}

    # the watched set folds a non-null monitor: the root Director @180 and every
    # ordinary member @720. The monitoring member is the unenrolled watcher, so
    # it folds null.
    assert members[director_id]["monitor"] is not None
    assert members[director_id]["monitor"]["enabled"] is True
    assert members[director_id]["monitor"]["interval_seconds"] == 180
    assert members[member_id]["monitor"] is not None
    assert members[member_id]["monitor"]["enabled"] is True
    assert members[member_id]["monitor"]["interval_seconds"] == 720
    assert members[watcher_id]["monitor"] is None


# --- GET /api/members/{id}/monitor ------------------------------------------


def test_get_member_monitor__200_for_director(api_db, client):
    # the root Director is enrolled @180
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]

    resp = client.get(f"/api/members/{director_id}/monitor", headers=_headers(sid))
    assert resp.status_code == 200, resp.text
    data = resp.json()
    assert data["interval_seconds"] == 180
    assert data["enabled"] is True
    assert "last_ping_at" in data


def test_get_member_monitor__200_for_member(api_db, client):
    # an ordinary member is enrolled @720
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    member_id = _register_member(fleet, name="alice")

    resp = client.get(f"/api/members/{member_id}/monitor", headers=_headers(sid))
    assert resp.status_code == 200, resp.text
    data = resp.json()
    assert data["interval_seconds"] == 720
    assert data["enabled"] is True
    assert "last_ping_at" in data


def test_get_member_monitor__404_for_monitoring_member_not_enrolled(api_db, client):
    # the monitoring member is the unenrolled watcher → its monitor endpoint 404s
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    watcher_id = _register_monitoring_member(fleet, pane_id="%7")["member_id"]

    resp = client.get(f"/api/members/{watcher_id}/monitor", headers=_headers(sid))
    assert resp.status_code == 404, resp.text


def test_get_member_monitor__404_for_unknown_member(api_db, client):
    sid = _create_fleet()["fleet_id"]
    resp = client.get("/api/members/999999/monitor", headers=_headers(sid))
    assert resp.status_code == 404, resp.text


# --- PATCH /api/members/{id}/monitor ----------------------------------------


def test_patch_member_monitor__updates_member_interval_and_enabled(api_db, client):
    # an ordinary member is enrolled @720 and editable via the API
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    member_id = _register_member(fleet, name="alice")

    resp = client.patch(
        f"/api/members/{member_id}/monitor",
        headers=_headers(sid),
        json={"interval_seconds": 30, "enabled": False},
    )
    assert resp.status_code == 200, resp.text
    data = resp.json()
    assert data["interval_seconds"] == 30
    assert data["enabled"] is False

    cfg = broker.get_monitor_config(sid, member_id)
    assert cfg["interval_seconds"] == 30
    assert cfg["enabled"] is False


def test_patch_member_monitor__422_on_interval_below_one(api_db, client):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    member_id = _register_member(fleet, name="alice")

    resp = client.patch(
        f"/api/members/{member_id}/monitor",
        headers=_headers(sid),
        json={"interval_seconds": 0},
    )
    assert resp.status_code == 422, resp.text


def test_patch_member_monitor__404_on_not_enrolled(api_db, client):
    # the monitoring member is the unenrolled watcher → its monitor edit 404s
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    watcher_id = _register_monitoring_member(fleet, pane_id="%7")["member_id"]

    resp = client.patch(
        f"/api/members/{watcher_id}/monitor",
        headers=_headers(sid),
        json={"interval_seconds": 30},
    )
    assert resp.status_code == 404, resp.text


def test_patch_member_monitor__404_on_unknown_member(api_db, client):
    sid = _create_fleet()["fleet_id"]
    resp = client.patch(
        "/api/members/999999/monitor",
        headers=_headers(sid),
        json={"interval_seconds": 30},
    )
    assert resp.status_code == 404, resp.text


def test_patch_member_monitor__404_not_500_when_update_raises(
    api_db, tmp_path, monkeypatch
):
    # TOCTOU race: the member passes the initial enrolled check, then
    # update_monitor_config raises a ClickException (e.g. deregistered mid-call).
    # The endpoint must surface 404, not let the ClickException become a 500.
    # The target must be enrolled to pass the pre-check, so use an ordinary
    # member (a watched member enrolled @720).
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    member_id = _register_member(fleet, name="alice")

    def boom(*args, **kwargs):
        raise click.ClickException("member not enrolled")

    monkeypatch.setattr(broker, "update_monitor_config", boom)

    # raise_server_exceptions=False so an unhandled 500 returns as a response
    # (clean 500 != 404 assertion) rather than being re-raised before the fix.
    client = TestClient(
        create_app(str(tmp_path / "nodist")), raise_server_exceptions=False
    )
    resp = client.patch(
        f"/api/members/{member_id}/monitor",
        headers=_headers(sid),
        json={"interval_seconds": 30},
    )
    assert resp.status_code == 404, resp.text
