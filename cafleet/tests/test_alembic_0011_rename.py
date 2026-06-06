"""Migration 0011 (``sessions`` → ``fleets``) upgrade/downgrade round-trip.

This test drives the real Alembic migration chain against an ISOLATED
tempfile SQLite DB (never the live broker DB). It validates the four
guarantees of the ``0011_rename_sessions_to_fleets`` revision:

(a) row preservation across the table/column rename,
(b) the schema round-trip (``fleets`` table, ``fleet_id`` PK/FK columns,
    ``idx_agents_fleet_status`` index) on upgrade and its full inverse on
    downgrade,
(c) ``PRAGMA foreign_key_check`` reports zero violations after BOTH
    directions, and
(d) the ``agents`` FK target auto-propagates to ``fleets(fleet_id)`` on
    upgrade and back to ``sessions(session_id)`` on downgrade — the
    SQLite >= 3.25 native-RENAME behavior the migration relies on (Risk 4).
    This is asserted directly, not assumed.

Old-name retention (deliberate): a migration round-trip test is the one
place under ``cafleet/tests`` that MUST name the pre-0011 ``sessions`` table
and ``session_id`` columns — the migration's contract is precisely "rename
forward on upgrade, restore the original schema on downgrade", so the
downgrade assertions necessarily reference the old names. This is the same
category as the immutable Alembic revisions ``0001``-``0010``: it describes
schema history, not the current live concept. It is NOT a missed rename.

The global ``Engine.connect`` listener registered by importing
``cafleet.db.engine`` (via ``conftest.py``) applies ``PRAGMA
foreign_keys=ON`` to every connection — including Alembic's migration
engine — so this test exercises the rename under the same FK enforcement
that production migrations run under.
"""

import importlib.resources

import pytest
from alembic import command
from alembic.config import Config
from sqlalchemy import create_engine, inspect, text

SEED_FLEET_ID = "fleet-0011-test"
SEED_DIRECTOR_ID = "agent-director-0011"
SEED_MEMBER_ID = "agent-member-0011"
SEED_LABEL = "migration-0011-fixture"
SEED_TS = "2026-01-01T00:00:00.000000+00:00"


def _run_alembic(db_path, action: str, revision: str) -> None:
    """Run a single ``command.upgrade``/``command.downgrade`` against ``db_path``.

    The bundled ``alembic.ini`` is materialized to a real path because a
    zipped-wheel install returns a Traversable Alembic cannot open; the
    context is held open across the command (mirrors ``cafleet db init``).
    """
    with importlib.resources.as_file(
        importlib.resources.files("cafleet") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{db_path}")
        getattr(command, action)(cfg, revision)


def _seed_pre_0011(db_path) -> None:
    """Seed one CAFleet entity + two agents under the pre-0011 schema.

    Insert order satisfies the circular FK under ``foreign_keys=ON``:
    the parent row first with a NULL director, then the agents that
    reference it, then backfill the director FK.
    """
    engine = create_engine(f"sqlite:///{db_path}")
    try:
        with engine.begin() as conn:
            conn.execute(
                text(
                    "INSERT INTO sessions "
                    "(session_id, label, created_at, deleted_at, director_agent_id) "
                    "VALUES (:sid, :label, :created_at, NULL, NULL)"
                ),
                {"sid": SEED_FLEET_ID, "label": SEED_LABEL, "created_at": SEED_TS},
            )
            for agent_id, name, description in (
                (SEED_DIRECTOR_ID, "Director", "Root Director for this fleet"),
                (SEED_MEMBER_ID, "Tester", "Member agent"),
            ):
                conn.execute(
                    text(
                        "INSERT INTO agents "
                        "(agent_id, session_id, name, description, status, "
                        "registered_at, deregistered_at, agent_card_json) "
                        "VALUES (:aid, :sid, :name, :desc, 'active', :ts, NULL, '{}')"
                    ),
                    {
                        "aid": agent_id,
                        "sid": SEED_FLEET_ID,
                        "name": name,
                        "desc": description,
                        "ts": SEED_TS,
                    },
                )
            conn.execute(
                text(
                    "UPDATE sessions SET director_agent_id = :did "
                    "WHERE session_id = :sid"
                ),
                {"did": SEED_DIRECTOR_ID, "sid": SEED_FLEET_ID},
            )
    finally:
        engine.dispose()


def _table_names(db_path) -> set[str]:
    engine = create_engine(f"sqlite:///{db_path}")
    try:
        return set(inspect(engine).get_table_names())
    finally:
        engine.dispose()


def _column_names(db_path, table: str) -> set[str]:
    engine = create_engine(f"sqlite:///{db_path}")
    try:
        return {col["name"] for col in inspect(engine).get_columns(table)}
    finally:
        engine.dispose()


def _index_columns(db_path, table: str) -> dict[str, list[str]]:
    engine = create_engine(f"sqlite:///{db_path}")
    try:
        return {
            idx["name"]: list(idx["column_names"])
            for idx in inspect(engine).get_indexes(table)
        }
    finally:
        engine.dispose()


def _foreign_keys(db_path, table: str) -> list[dict]:
    engine = create_engine(f"sqlite:///{db_path}")
    try:
        return inspect(engine).get_foreign_keys(table)
    finally:
        engine.dispose()


def _foreign_key_check(db_path) -> list:
    engine = create_engine(f"sqlite:///{db_path}")
    try:
        with engine.connect() as conn:
            return conn.execute(text("PRAGMA foreign_key_check")).fetchall()
    finally:
        engine.dispose()


def _scalar(db_path, sql: str, **params):
    engine = create_engine(f"sqlite:///{db_path}")
    try:
        with engine.connect() as conn:
            return conn.execute(text(sql), params).scalar()
    finally:
        engine.dispose()


def _rows(db_path, sql: str, **params) -> list:
    engine = create_engine(f"sqlite:///{db_path}")
    try:
        with engine.connect() as conn:
            return conn.execute(text(sql), params).fetchall()
    finally:
        engine.dispose()


@pytest.fixture
def seeded_0010_db(tmp_path):
    """Tempfile DB migrated to 0010 (pre-rename) and seeded with rows."""
    db_path = tmp_path / "migration_0011.db"
    _run_alembic(db_path, "upgrade", "0010")
    _seed_pre_0011(db_path)
    return db_path


@pytest.fixture
def upgraded_0011_db(seeded_0010_db):
    """Seeded DB advanced through the 0011 upgrade."""
    _run_alembic(seeded_0010_db, "upgrade", "0011")
    return seeded_0010_db


@pytest.fixture
def roundtripped_0010_db(upgraded_0011_db):
    """Upgraded DB taken back down to 0010 via the 0011 downgrade."""
    _run_alembic(upgraded_0011_db, "downgrade", "0010")
    return upgraded_0011_db


# --- Upgrade direction ------------------------------------------------------


def test_upgrade_renames_schema_to_fleets(upgraded_0011_db):
    db = upgraded_0011_db

    tables = _table_names(db)
    assert "fleets" in tables
    assert "sessions" not in tables

    fleet_cols = _column_names(db, "fleets")
    assert "fleet_id" in fleet_cols
    assert "session_id" not in fleet_cols

    agent_cols = _column_names(db, "agents")
    assert "fleet_id" in agent_cols
    assert "session_id" not in agent_cols

    indexes = _index_columns(db, "agents")
    assert "idx_agents_fleet_status" in indexes
    assert "idx_agents_session_status" not in indexes
    assert indexes["idx_agents_fleet_status"] == ["fleet_id", "status"]


def test_upgrade_preserves_all_rows(upgraded_0011_db):
    db = upgraded_0011_db

    assert _scalar(db, "SELECT COUNT(*) FROM fleets") == 1
    assert _scalar(db, "SELECT COUNT(*) FROM agents") == 2

    fleet = _rows(db, "SELECT fleet_id, label, director_agent_id FROM fleets")[0]
    assert fleet[0] == SEED_FLEET_ID
    assert fleet[1] == SEED_LABEL
    assert fleet[2] == SEED_DIRECTOR_ID

    member_ids = {
        row[0]
        for row in _rows(
            db, "SELECT agent_id FROM agents WHERE fleet_id = :fid", fid=SEED_FLEET_ID
        )
    }
    assert member_ids == {SEED_DIRECTOR_ID, SEED_MEMBER_ID}


def test_upgrade_agents_fk_targets_fleets(upgraded_0011_db):
    fks = _foreign_keys(upgraded_0011_db, "agents")
    assert len(fks) == 1
    fk = fks[0]
    assert fk["constrained_columns"] == ["fleet_id"]
    assert fk["referred_table"] == "fleets"
    assert fk["referred_columns"] == ["fleet_id"]


def test_upgrade_foreign_key_check_reports_no_violations(upgraded_0011_db):
    assert _foreign_key_check(upgraded_0011_db) == []


# --- Downgrade direction (full inverse) -------------------------------------


def test_downgrade_restores_original_schema(roundtripped_0010_db):
    db = roundtripped_0010_db

    tables = _table_names(db)
    assert "sessions" in tables
    assert "fleets" not in tables

    session_cols = _column_names(db, "sessions")
    assert "session_id" in session_cols
    assert "fleet_id" not in session_cols

    agent_cols = _column_names(db, "agents")
    assert "session_id" in agent_cols
    assert "fleet_id" not in agent_cols

    indexes = _index_columns(db, "agents")
    assert "idx_agents_session_status" in indexes
    assert "idx_agents_fleet_status" not in indexes
    assert indexes["idx_agents_session_status"] == ["session_id", "status"]


def test_downgrade_preserves_all_rows(roundtripped_0010_db):
    db = roundtripped_0010_db

    assert _scalar(db, "SELECT COUNT(*) FROM sessions") == 1
    assert _scalar(db, "SELECT COUNT(*) FROM agents") == 2

    session = _rows(db, "SELECT session_id, label, director_agent_id FROM sessions")[0]
    assert session[0] == SEED_FLEET_ID
    assert session[1] == SEED_LABEL
    assert session[2] == SEED_DIRECTOR_ID

    member_ids = {
        row[0]
        for row in _rows(
            db,
            "SELECT agent_id FROM agents WHERE session_id = :sid",
            sid=SEED_FLEET_ID,
        )
    }
    assert member_ids == {SEED_DIRECTOR_ID, SEED_MEMBER_ID}


def test_downgrade_agents_fk_targets_sessions(roundtripped_0010_db):
    fks = _foreign_keys(roundtripped_0010_db, "agents")
    assert len(fks) == 1
    fk = fks[0]
    assert fk["constrained_columns"] == ["session_id"]
    assert fk["referred_table"] == "sessions"
    assert fk["referred_columns"] == ["session_id"]


def test_downgrade_foreign_key_check_reports_no_violations(roundtripped_0010_db):
    assert _foreign_key_check(roundtripped_0010_db) == []
