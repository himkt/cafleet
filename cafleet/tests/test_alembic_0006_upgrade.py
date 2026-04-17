"""Tests for Alembic migration ``0006_seed_administrator_agent`` (design 0000025 §C)."""

import importlib.resources
import json
import uuid
from datetime import UTC, datetime

import pytest
from alembic import command
from alembic.config import Config
from sqlalchemy import create_engine, text


def _make_alembic_cfg(db_path) -> Config:
    with importlib.resources.as_file(
        importlib.resources.files("cafleet") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{db_path}")
        return cfg


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()


def _seed_session(
    engine, *, session_id: str, created_at: str, label: str | None = None
):
    with engine.begin() as conn:
        conn.execute(
            text(
                "INSERT INTO sessions (session_id, label, created_at) "
                "VALUES (:sid, :label, :created_at)"
            ),
            {"sid": session_id, "label": label, "created_at": created_at},
        )


def _seed_user_agent(
    engine,
    *,
    agent_id: str,
    session_id: str,
    name: str,
    registered_at: str,
    status: str = "active",
):
    """INSERT a regular (non-Administrator) agent row at the 0005 schema level."""
    card = json.dumps({"name": name, "description": "test user agent", "skills": []})
    with engine.begin() as conn:
        conn.execute(
            text(
                "INSERT INTO agents "
                "(agent_id, session_id, name, description, status, "
                " registered_at, deregistered_at, agent_card_json) "
                "VALUES (:aid, :sid, :name, :desc, :status, :at, NULL, :card)"
            ),
            {
                "aid": agent_id,
                "sid": session_id,
                "name": name,
                "desc": "test user agent",
                "status": status,
                "at": registered_at,
                "card": card,
            },
        )


def _fetch_administrator_rows(engine, session_id: str) -> list[dict]:
    """Return all administrator rows (per cafleet.kind) for a session as dicts."""
    with engine.connect() as conn:
        rows = conn.execute(
            text(
                "SELECT agent_id, session_id, name, description, status, "
                "       registered_at, agent_card_json "
                "FROM agents "
                "WHERE session_id = :sid "
                "  AND json_extract(agent_card_json, '$.cafleet.kind') "
                "      = 'builtin-administrator'"
            ),
            {"sid": session_id},
        ).fetchall()
    return [
        {
            "agent_id": r[0],
            "session_id": r[1],
            "name": r[2],
            "description": r[3],
            "status": r[4],
            "registered_at": r[5],
            "agent_card_json": r[6],
        }
        for r in rows
    ]


@pytest.fixture
def db_at_0005(tmp_path):
    """Upgrade a fresh DB to revision ``0005`` (pre-seed state) and return its path."""
    db_path = tmp_path / "upgrade_0006_test.db"
    cfg = _make_alembic_cfg(db_path)
    command.upgrade(cfg, "0005")
    return db_path


class TestMigration0006UpgradeSeed:
    def test_seeds_one_administrator_per_session(self, db_at_0005):
        from cafleet.broker import ADMINISTRATOR_KIND

        sid_a = str(uuid.uuid4())
        sid_b = str(uuid.uuid4())
        created_at_a = "2026-01-01T00:00:00+00:00"
        created_at_b = "2026-02-02T12:34:56+00:00"

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            _seed_session(engine, session_id=sid_a, created_at=created_at_a, label="a")
            _seed_session(engine, session_id=sid_b, created_at=created_at_b, label="b")
            _seed_user_agent(
                engine,
                agent_id=str(uuid.uuid4()),
                session_id=sid_a,
                name="user-1",
                registered_at=created_at_a,
            )
            _seed_user_agent(
                engine,
                agent_id=str(uuid.uuid4()),
                session_id=sid_a,
                name="user-2",
                registered_at=created_at_a,
            )
            # Session B has no user agents — it must still gain an Administrator.
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0005)
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            admins_a = _fetch_administrator_rows(engine, sid_a)
            admins_b = _fetch_administrator_rows(engine, sid_b)

            assert len(admins_a) == 1
            assert len(admins_b) == 1

            for admins, expected_created_at in (
                (admins_a, created_at_a),
                (admins_b, created_at_b),
            ):
                row = admins[0]
                assert row["name"] == "Administrator"
                assert row["status"] == "active"
                assert row["registered_at"] == expected_created_at
                uuid.UUID(row["agent_id"])
                card = json.loads(row["agent_card_json"])
                assert card["cafleet"]["kind"] == ADMINISTRATOR_KIND
                assert card["name"] == "Administrator"
                assert card["skills"] == []
        finally:
            engine.dispose()

    def test_seed_preserves_existing_user_agents(self, db_at_0005):
        sid = str(uuid.uuid4())
        created_at = "2026-01-01T00:00:00+00:00"
        user_id = str(uuid.uuid4())

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            _seed_session(engine, session_id=sid, created_at=created_at)
            _seed_user_agent(
                engine,
                agent_id=user_id,
                session_id=sid,
                name="survivor",
                registered_at=created_at,
            )
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0005)
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            with engine.connect() as conn:
                row = conn.execute(
                    text("SELECT name, status FROM agents WHERE agent_id = :aid"),
                    {"aid": user_id},
                ).fetchone()
            assert row is not None
            assert row[0] == "survivor"
            assert row[1] == "active"
        finally:
            engine.dispose()

    def test_no_sessions_no_administrators(self, db_at_0005):
        cfg = _make_alembic_cfg(db_at_0005)
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            with engine.connect() as conn:
                (count,) = conn.execute(
                    text(
                        "SELECT COUNT(*) FROM agents "
                        "WHERE json_extract(agent_card_json, '$.cafleet.kind') "
                        "      = 'builtin-administrator'"
                    )
                ).fetchone()
            assert count == 0
        finally:
            engine.dispose()


class TestMigration0006UpgradeIdempotent:
    def test_double_upgrade_is_idempotent(self, db_at_0005):
        sid_a = str(uuid.uuid4())
        sid_b = str(uuid.uuid4())
        created_at = "2026-03-03T03:03:03+00:00"

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            _seed_session(engine, session_id=sid_a, created_at=created_at)
            _seed_session(engine, session_id=sid_b, created_at=created_at)
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0005)
        command.upgrade(cfg, "head")
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            admins_a = _fetch_administrator_rows(engine, sid_a)
            admins_b = _fetch_administrator_rows(engine, sid_b)
            assert len(admins_a) == 1
            assert len(admins_b) == 1
        finally:
            engine.dispose()

    def test_idempotent_preserves_original_administrator_id(self, db_at_0005):
        sid = str(uuid.uuid4())
        created_at = "2026-04-04T04:04:04+00:00"

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            _seed_session(engine, session_id=sid, created_at=created_at)
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0005)
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            first_admins = _fetch_administrator_rows(engine, sid)
            assert len(first_admins) == 1
            first_id = first_admins[0]["agent_id"]
        finally:
            engine.dispose()

        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            second_admins = _fetch_administrator_rows(engine, sid)
            assert len(second_admins) == 1
            assert second_admins[0]["agent_id"] == first_id
        finally:
            engine.dispose()


class TestMigration0006DowngradeSmoke:
    """Only tests task-free sessions. The non-empty case is out of scope:
    ``tasks.context_id`` uses ``ON DELETE RESTRICT``, so downgrading with
    existing task references would test SQLite FK enforcement, not our migration.
    """

    def test_downgrade_removes_administrator_on_empty_session(self, db_at_0005):
        sid = str(uuid.uuid4())
        created_at = "2026-05-05T05:05:05+00:00"

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            _seed_session(engine, session_id=sid, created_at=created_at)
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0005)
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            admins = _fetch_administrator_rows(engine, sid)
            assert len(admins) == 1
        finally:
            engine.dispose()

        command.downgrade(cfg, "0005")

        engine = create_engine(f"sqlite:///{db_at_0005}")
        try:
            admins = _fetch_administrator_rows(engine, sid)
            assert len(admins) == 0

            with engine.connect() as conn:
                (version,) = conn.execute(
                    text("SELECT version_num FROM alembic_version")
                ).fetchone()
            assert version == "0005"
        finally:
            engine.dispose()
