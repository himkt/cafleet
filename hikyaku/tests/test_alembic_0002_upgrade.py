"""Tests for Alembic migration ``0002_local_simplification``.

Verifies the upgrade path from the 0001 schema (with ``api_keys`` + ``agents.tenant_id``)
to the 0002 schema (with ``sessions`` + ``agents.session_id``):

  1. ``sessions`` table is created and seeded from ``api_keys`` rows —
     each ``api_key_hash`` becomes a ``session_id`` verbatim.
  2. ``agents.tenant_id`` is renamed to ``session_id`` and its FK retargets
     ``sessions.session_id``.
  3. ``api_keys`` table is dropped entirely.
  4. Index ``idx_agents_tenant_status`` is replaced by ``idx_agents_session_status``.
  5. ``downgrade()`` raises ``NotImplementedError`` — this is a one-way migration.

Design doc reference: design-docs/0000015-remove-auth0-local-session-model/design-doc.md
Specification §2 (Alembic Migration ``0002_local_simplification``).

Test isolation strategy:

  Each test creates its own temporary DB file and runs Alembic migrations
  via ``command.upgrade``. The DB is initialized to the ``0001`` revision
  first, then test data is inserted, and the upgrade to ``0002_local_simplification``
  is applied. This ensures the migration works against a realistic schema state.
"""

import importlib.resources
from datetime import UTC, datetime

import pytest
from alembic import command
from alembic.config import Config
from sqlalchemy import create_engine, inspect, text


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_alembic_cfg(db_path) -> Config:
    """Create an Alembic Config pointing at the given SQLite DB file."""
    with importlib.resources.as_file(
        importlib.resources.files("hikyaku") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{db_path}")
        return cfg


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def db_at_0001(tmp_path):
    """Create a DB at Alembic revision 0001 and return the path.

    Uses ``command.upgrade(cfg, "0001")`` to reach the base schema with
    ``api_keys`` and ``agents.tenant_id`` columns in place.
    """
    db_path = tmp_path / "upgrade_test.db"
    cfg = _make_alembic_cfg(db_path)
    command.upgrade(cfg, "0001")
    return db_path


# ---------------------------------------------------------------------------
# Migration upgrade tests
# ---------------------------------------------------------------------------


class TestMigration0002Upgrade:
    """Tests for the 0002_local_simplification upgrade path."""

    def test_sessions_table_created(self, db_at_0001):
        """After upgrade, ``sessions`` table exists with expected columns."""
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
        """After upgrade, ``api_keys`` table no longer exists."""
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
        """After upgrade, ``agents`` has ``session_id`` and no ``tenant_id``."""
        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            insp = inspect(engine)
            cols = {col["name"] for col in insp.get_columns("agents")}
            assert "session_id" in cols, (
                "agents table does not have session_id column after migration"
            )
            assert "tenant_id" not in cols, (
                "agents table still has tenant_id column after migration — "
                "rename did not apply"
            )
        finally:
            engine.dispose()

    def test_session_seeded_from_api_key(self, db_at_0001):
        """Migration seeds a session row per api_keys row, using api_key_hash
        as the session_id."""
        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            # Insert an api_key at the 0001 schema level
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

        # Run migration
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
        """An agent referencing an api_key via tenant_id has a valid
        session_id FK after the migration."""
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
                # Verify agent now has session_id = the old api_key_hash
                row = conn.execute(
                    text("SELECT session_id FROM agents WHERE agent_id = :aid"),
                    {"aid": agent_id},
                ).fetchone()
                assert row is not None
                assert row[0] == api_key_hash

                # Verify the FK is valid: session row exists
                sess_row = conn.execute(
                    text("SELECT session_id FROM sessions WHERE session_id = :sid"),
                    {"sid": api_key_hash},
                ).fetchone()
                assert sess_row is not None
        finally:
            engine.dispose()

    def test_revoked_api_key_also_seeded(self, db_at_0001):
        """Migration seeds sessions from ALL api_keys rows — including revoked ones.

        This prevents FK violations for agents whose tenant_id referenced a
        revoked key. See design doc §2 Note about seeding step.
        """
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
            assert active_hash in session_ids, (
                "Active api_key was not seeded into sessions"
            )
            assert revoked_hash in session_ids, (
                "Revoked api_key was not seeded into sessions — migration "
                "must seed ALL api_keys rows to prevent FK violations"
            )
        finally:
            engine.dispose()

    def test_fresh_db_upgrade_no_op_on_empty_api_keys(self, db_at_0001):
        """On a fresh DB with zero api_keys rows, the INSERT ... SELECT is a
        no-op and the migration still succeeds."""
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
        """After upgrade, ``idx_agents_session_status`` index exists on
        ``(session_id, status)``."""
        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            insp = inspect(engine)
            indexes = insp.get_indexes("agents")
            match = [
                idx for idx in indexes if idx["name"] == "idx_agents_session_status"
            ]
            assert len(match) == 1, (
                f"expected idx_agents_session_status, got: {[i['name'] for i in indexes]}"
            )
            assert "session_id" in match[0]["column_names"]
            assert "status" in match[0]["column_names"]
        finally:
            engine.dispose()

    def test_idx_agents_tenant_status_gone(self, db_at_0001):
        """After upgrade, ``idx_agents_tenant_status`` no longer exists."""
        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        engine = create_engine(f"sqlite:///{db_at_0001}")
        try:
            insp = inspect(engine)
            indexes = insp.get_indexes("agents")
            old = [idx for idx in indexes if idx["name"] == "idx_agents_tenant_status"]
            assert len(old) == 0, (
                "idx_agents_tenant_status still exists after migration — "
                "batch_alter_table should have replaced it with idx_agents_session_status"
            )
        finally:
            engine.dispose()

    def test_alembic_version_updated(self, db_at_0001):
        """After upgrade, alembic_version row reflects 0002_local_simplification."""
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


# ---------------------------------------------------------------------------
# Downgrade test
# ---------------------------------------------------------------------------


class TestMigration0002Downgrade:
    """Tests for the 0002_local_simplification downgrade (must raise)."""

    def test_downgrade_raises_not_implemented(self, db_at_0001):
        """Attempting to downgrade from 0002 raises an error.

        The migration's downgrade() raises NotImplementedError. Alembic
        surfaces this as a CommandError or similar failure — the key
        assertion is that the downgrade does not succeed silently.
        """
        cfg = _make_alembic_cfg(db_at_0001)
        command.upgrade(cfg, "0002_local_simplification")

        with pytest.raises(Exception):
            command.downgrade(cfg, "0001")
