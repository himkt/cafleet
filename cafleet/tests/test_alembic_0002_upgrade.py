"""Tests for Alembic migration ``0002_local_simplification`` (design 0000015)."""

import importlib.resources
from datetime import UTC, datetime

import pytest
from alembic import command
from alembic.config import Config
from sqlalchemy import create_engine, inspect, text


def _make_alembic_cfg(db_path) -> Config:
    with importlib.resources.as_file(
        importlib.resources.files("cafleet") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{db_path}")
        return cfg


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()


@pytest.fixture
def db_at_0001(tmp_path):
    """Upgrade a fresh DB to revision ``0001`` and return its path."""
    db_path = tmp_path / "upgrade_test.db"
    cfg = _make_alembic_cfg(db_path)
    command.upgrade(cfg, "0001")
    return db_path


class TestMigration0002Upgrade:
    def test_sessions_table_created(self, db_at_0001):
        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            insp = inspect(engine)
            tables = set(insp.get_table_names())
            assert "sessions" in tables

            cols = {col["name"] for col in insp.get_columns("sessions")}
            assert {"session_id", "label", "created_at"} <= cols
        finally:
            engine.dispose()

    def test_api_keys_table_dropped(self, db_at_0001):
        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            insp = inspect(engine)
            tables = set(insp.get_table_names())
            assert "api_keys" not in tables
        finally:
            engine.dispose()

    def test_agents_column_renamed_to_session_id(self, db_at_0001):
        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            insp = inspect(engine)
            cols = {col["name"] for col in insp.get_columns("agents")}
            assert "session_id" in cols
            assert "tenant_id" not in cols
        finally:
            engine.dispose()

    def test_session_seeded_from_api_key(self, db_at_0001):
        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            with engine.begin() as conn:
                conn.execute(
                    text(
                        "INSERT INTO api_keys "
                        "(api_key_hash, owner_sub, key_prefix, status, created_at) "
                        "VALUES (:hash, :owner, :prefix, :status, :created)"
                    ),
                    {
                        "hash": "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
                        "owner": "auth0|test-owner",
                        "prefix": "hky_abcd",
                        "status": "active",
                        "created": _now_iso(),
                    },
                )
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            with engine.connect() as conn:
                rows = conn.execute(
                    text("SELECT session_id, label, created_at FROM sessions")
                ).fetchall()

            assert len(rows) == 1
            session_id, label, created_at = rows[0]
            assert (
                session_id
                == "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
            )
            assert label == "legacy-hky_abcd"
            assert created_at is not None
        finally:
            engine.dispose()

    def test_agent_fk_valid_after_upgrade(self, db_at_0001):
        api_key_hash = (
            "beef0000beef0000beef0000beef0000beef0000beef0000beef0000beef0000"
        )
        agent_id = "agent-migration-test"
        now = _now_iso()

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            with engine.begin() as conn:
                conn.execute(
                    text(
                        "INSERT INTO api_keys "
                        "(api_key_hash, owner_sub, key_prefix, status, created_at) "
                        "VALUES (:hash, :owner, :prefix, :status, :created)"
                    ),
                    {
                        "hash": api_key_hash,
                        "owner": "auth0|fk-test",
                        "prefix": "hky_beef",
                        "status": "active",
                        "created": now,
                    },
                )
                conn.execute(
                    text(
                        "INSERT INTO agents "
                        "(agent_id, tenant_id, name, description, status, "
                        " registered_at, agent_card_json) "
                        "VALUES (:aid, :tid, :name, :desc, :status, :at, :card)"
                    ),
                    {
                        "aid": agent_id,
                        "tid": api_key_hash,
                        "name": "Test",
                        "desc": "Test agent",
                        "status": "active",
                        "at": now,
                        "card": "{}",
                    },
                )
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            with engine.connect() as conn:
                row = conn.execute(
                    text("SELECT session_id FROM agents WHERE agent_id = :aid"),
                    {"aid": agent_id},
                ).fetchone()
                assert row is not None
                assert row[0] == api_key_hash

                sess_row = conn.execute(
                    text("SELECT session_id FROM sessions WHERE session_id = :sid"),
                    {"sid": api_key_hash},
                ).fetchone()
                assert sess_row is not None
        finally:
            engine.dispose()

    def test_revoked_api_key_also_seeded(self, db_at_0001):
        """Revoked keys must also be seeded to prevent FK violations for agents
        whose tenant_id referenced a revoked key."""
        active_hash = "aaaa0000" * 8
        revoked_hash = "bbbb0000" * 8
        now = _now_iso()

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            with engine.begin() as conn:
                conn.execute(
                    text(
                        "INSERT INTO api_keys "
                        "(api_key_hash, owner_sub, key_prefix, status, created_at) "
                        "VALUES (:hash, :owner, :prefix, :status, :created)"
                    ),
                    {
                        "hash": active_hash,
                        "owner": "auth0|owner-active",
                        "prefix": "hky_aaaa",
                        "status": "active",
                        "created": now,
                    },
                )
                conn.execute(
                    text(
                        "INSERT INTO api_keys "
                        "(api_key_hash, owner_sub, key_prefix, status, created_at) "
                        "VALUES (:hash, :owner, :prefix, :status, :created)"
                    ),
                    {
                        "hash": revoked_hash,
                        "owner": "auth0|owner-revoked",
                        "prefix": "hky_bbbb",
                        "status": "revoked",
                        "created": now,
                    },
                )
        finally:
            engine.dispose()

        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            with engine.connect() as conn:
                rows = conn.execute(
                    text("SELECT session_id FROM sessions ORDER BY session_id")
                ).fetchall()
            session_ids = {row[0] for row in rows}
            assert active_hash in session_ids
            assert revoked_hash in session_ids
        finally:
            engine.dispose()

    def test_fresh_db_upgrade_no_op_on_empty_api_keys(self, db_at_0001):
        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            with engine.connect() as conn:
                rows = conn.execute(text("SELECT COUNT(*) FROM sessions")).fetchone()
            assert rows[0] == 0
        finally:
            engine.dispose()

    def test_idx_agents_session_status_exists(self, db_at_0001):
        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            insp = inspect(engine)
            indexes = insp.get_indexes("agents")
            match = [
                idx for idx in indexes if idx["name"] == "idx_agents_session_status"
            ]
            assert len(match) == 1
            assert "session_id" in match[0]["column_names"]
            assert "status" in match[0]["column_names"]
        finally:
            engine.dispose()

    def test_idx_agents_tenant_status_gone(self, db_at_0001):
        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            insp = inspect(engine)
            indexes = insp.get_indexes("agents")
            old = [idx for idx in indexes if idx["name"] == "idx_agents_tenant_status"]
            assert len(old) == 0
        finally:
            engine.dispose()

    def test_alembic_version_updated(self, db_at_0001):
        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            with engine.connect() as conn:
                rows = conn.execute(
                    text("SELECT version_num FROM alembic_version")
                ).fetchall()
            assert len(rows) == 1
            assert rows[0][0] == "0002_local_simplification"
        finally:
            engine.dispose()


class TestMigration0002Downgrade:
    def test_downgrade_raises_not_implemented(self, db_at_0001):
        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        with pytest.raises(NotImplementedError, match="one-way migration"):
            command.downgrade(cfg, "0001")
