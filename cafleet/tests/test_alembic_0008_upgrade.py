"""Tests for Alembic migration ``0008_capitalize_root_director_name``."""

import importlib.resources
import json
import uuid

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


def _insert_session_with_director(
    engine,
    *,
    session_id: str,
    created_at: str,
    director_agent_id: str,
    director_name: str = "director",
    label: str | None = None,
):
    card = json.dumps(
        {"name": director_name, "description": "Root Director", "skills": []}
    )
    with engine.begin() as conn:
        conn.execute(
            text(
                "INSERT INTO sessions "
                "(session_id, label, created_at, deleted_at, director_agent_id) "
                "VALUES (:sid, :label, :created_at, NULL, NULL)"
            ),
            {"sid": session_id, "label": label, "created_at": created_at},
        )
        conn.execute(
            text(
                "INSERT INTO agents "
                "(agent_id, session_id, name, description, status, "
                " registered_at, deregistered_at, agent_card_json) "
                "VALUES (:aid, :sid, :name, :desc, 'active', "
                " :at, NULL, :card)"
            ),
            {
                "aid": director_agent_id,
                "sid": session_id,
                "name": director_name,
                "desc": "Root Director",
                "at": created_at,
                "card": card,
            },
        )
        conn.execute(
            text(
                "UPDATE sessions SET director_agent_id = :aid WHERE session_id = :sid"
            ),
            {"aid": director_agent_id, "sid": session_id},
        )


def _insert_user_agent(
    engine,
    *,
    agent_id: str,
    session_id: str,
    name: str,
    registered_at: str,
):
    card = json.dumps({"name": name, "description": "user agent", "skills": []})
    with engine.begin() as conn:
        conn.execute(
            text(
                "INSERT INTO agents "
                "(agent_id, session_id, name, description, status, "
                " registered_at, deregistered_at, agent_card_json) "
                "VALUES (:aid, :sid, :name, 'user agent', 'active', "
                " :at, NULL, :card)"
            ),
            {
                "aid": agent_id,
                "sid": session_id,
                "name": name,
                "at": registered_at,
                "card": card,
            },
        )


def _fetch_agent(engine, agent_id: str) -> tuple[str, str]:
    with engine.connect() as conn:
        row = conn.execute(
            text("SELECT name, agent_card_json FROM agents WHERE agent_id = :aid"),
            {"aid": agent_id},
        ).fetchone()
    assert row is not None
    return row[0], row[1]


@pytest.fixture
def db_at_0007(tmp_path):
    db_path = tmp_path / "upgrade_0008_test.db"
    cfg = _make_alembic_cfg(db_path)
    command.upgrade(cfg, "0007")
    return db_path


class TestMigration0008Upgrade:
    def test_renames_root_director_name_and_card(self, db_at_0007):
        sid = str(uuid.uuid4())
        director_id = str(uuid.uuid4())
        created_at = "2026-04-01T00:00:00+00:00"

        engine = create_engine(f"sqlite:///{db_at_0007}")
        try:
            _insert_session_with_director(
                engine,
                session_id=sid,
                created_at=created_at,
                director_agent_id=director_id,
            )
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0007)
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0007}")
        try:
            name, card_json = _fetch_agent(engine, director_id)
            assert name == "Director"
            card = json.loads(card_json)
            assert card["name"] == "Director"
        finally:
            engine.dispose()

    def test_leaves_user_agents_named_director_untouched(self, db_at_0007):
        sid = str(uuid.uuid4())
        director_id = str(uuid.uuid4())
        user_id = str(uuid.uuid4())
        created_at = "2026-04-02T00:00:00+00:00"

        engine = create_engine(f"sqlite:///{db_at_0007}")
        try:
            _insert_session_with_director(
                engine,
                session_id=sid,
                created_at=created_at,
                director_agent_id=director_id,
            )
            _insert_user_agent(
                engine,
                agent_id=user_id,
                session_id=sid,
                name="director-1",
                registered_at=created_at,
            )
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0007)
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0007}")
        try:
            user_name, user_card_json = _fetch_agent(engine, user_id)
            assert user_name == "director-1"
            user_card = json.loads(user_card_json)
            assert user_card["name"] == "director-1"
        finally:
            engine.dispose()

    def test_renames_director_across_multiple_sessions(self, db_at_0007):
        sid_a = str(uuid.uuid4())
        sid_b = str(uuid.uuid4())
        dir_a = str(uuid.uuid4())
        dir_b = str(uuid.uuid4())
        created_at = "2026-04-03T00:00:00+00:00"

        engine = create_engine(f"sqlite:///{db_at_0007}")
        try:
            _insert_session_with_director(
                engine,
                session_id=sid_a,
                created_at=created_at,
                director_agent_id=dir_a,
            )
            _insert_session_with_director(
                engine,
                session_id=sid_b,
                created_at=created_at,
                director_agent_id=dir_b,
            )
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0007)
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0007}")
        try:
            for aid in (dir_a, dir_b):
                name, card_json = _fetch_agent(engine, aid)
                assert name == "Director"
                assert json.loads(card_json)["name"] == "Director"
        finally:
            engine.dispose()


class TestMigration0008Idempotent:
    def test_double_upgrade_is_noop(self, db_at_0007):
        sid = str(uuid.uuid4())
        director_id = str(uuid.uuid4())
        created_at = "2026-04-04T00:00:00+00:00"

        engine = create_engine(f"sqlite:///{db_at_0007}")
        try:
            _insert_session_with_director(
                engine,
                session_id=sid,
                created_at=created_at,
                director_agent_id=director_id,
            )
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0007)
        command.upgrade(cfg, "head")
        command.upgrade(cfg, "head")

        engine = create_engine(f"sqlite:///{db_at_0007}")
        try:
            name, card_json = _fetch_agent(engine, director_id)
            assert name == "Director"
            assert json.loads(card_json)["name"] == "Director"
        finally:
            engine.dispose()


class TestMigration0008Downgrade:
    def test_downgrade_restores_lowercase_name(self, db_at_0007):
        sid = str(uuid.uuid4())
        director_id = str(uuid.uuid4())
        created_at = "2026-04-05T00:00:00+00:00"

        engine = create_engine(f"sqlite:///{db_at_0007}")
        try:
            _insert_session_with_director(
                engine,
                session_id=sid,
                created_at=created_at,
                director_agent_id=director_id,
            )
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0007)
        command.upgrade(cfg, "0008")
        command.downgrade(cfg, "0007")

        engine = create_engine(f"sqlite:///{db_at_0007}")
        try:
            name, card_json = _fetch_agent(engine, director_id)
            assert name == "director"
            assert json.loads(card_json)["name"] == "director"
        finally:
            engine.dispose()
