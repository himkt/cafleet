"""Alembic smoke test — runs real migrations against a tempfile DB.

Other tests use Base.metadata.create_all and bypass Alembic entirely,
so this is the only place that catches migration-vs-model drift.
"""

import importlib.resources

import pytest
from alembic import command
from alembic.config import Config
from alembic.script import ScriptDirectory
from sqlalchemy import create_engine, inspect, text


@pytest.fixture(scope="session")
def alembic_upgraded_db(tmp_path_factory):
    tmp_db_path = tmp_path_factory.mktemp("alembic_smoke") / "smoke.db"

    with importlib.resources.as_file(
        importlib.resources.files("cafleet") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{tmp_db_path}")
        command.upgrade(cfg, "head")

    return tmp_db_path


def test_alembic_upgrade_head_creates_expected_tables(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        tables = set(insp.get_table_names())

        expected = {
            "fleets",
            "agents",
            "tasks",
            "agent_placements",
            "alembic_version",
        }
        missing = expected - tables
        assert not missing
        assert "api_keys" not in tables
    finally:
        engine.dispose()


def test_alembic_version_table_records_collapsed_head_0001(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        with engine.connect() as conn:
            result = conn.execute(text("SELECT version_num FROM alembic_version"))
            rows = result.fetchall()
        assert rows == [("0001",)]
    finally:
        engine.dispose()


def test_only_one_migration_revision_exists():
    """The migration history collapses to a single schema-only initial revision."""
    with importlib.resources.as_file(
        importlib.resources.files("cafleet") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        script = ScriptDirectory.from_config(cfg)
        revisions = list(script.walk_revisions())

    assert len(revisions) == 1
    only = revisions[0]
    assert only.revision == "0001"
    assert only.down_revision is None


def test_minted_id_tables_declare_autoincrement(alembic_upgraded_db):
    """``fleets``/``agents``/``tasks`` mint ids via AUTOINCREMENT; placements do not."""
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        with engine.connect() as conn:
            # SQLite creates the sqlite_sequence table whenever an
            # AUTOINCREMENT table exists (guaranteeing ids are never reused).
            seq = conn.execute(
                text(
                    "SELECT name FROM sqlite_master "
                    "WHERE type='table' AND name='sqlite_sequence'"
                )
            ).fetchall()
            assert seq, "sqlite_sequence must exist for AUTOINCREMENT tables"

            for table in ("fleets", "agents", "tasks"):
                ddl = conn.execute(
                    text(
                        "SELECT sql FROM sqlite_master WHERE type='table' AND name=:t"
                    ),
                    {"t": table},
                ).scalar()
                assert "AUTOINCREMENT" in ddl.upper()

            # agent_placements.agent_id is the agents FK reused as a 1:1 PK —
            # explicitly NOT AUTOINCREMENT.
            placements_ddl = conn.execute(
                text(
                    "SELECT sql FROM sqlite_master "
                    "WHERE type='table' AND name='agent_placements'"
                )
            ).scalar()
            assert "AUTOINCREMENT" not in placements_ddl.upper()
    finally:
        engine.dispose()


@pytest.mark.parametrize(
    ("table", "pk_column"),
    [
        ("fleets", "fleet_id"),
        ("agents", "agent_id"),
        ("tasks", "task_id"),
        ("agent_placements", "agent_id"),
    ],
)
def test_primary_key_columns_are_integer(alembic_upgraded_db, table, pk_column):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        cols = {col["name"]: col for col in insp.get_columns(table)}
        assert "INT" in str(cols[pk_column]["type"]).upper()
    finally:
        engine.dispose()


def test_agent_placements_table_created_by_migration(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)

        tables = set(insp.get_table_names())
        assert "agent_placements" in tables

        cols = {col["name"]: col for col in insp.get_columns("agent_placements")}
        expected_cols = {
            "agent_id",
            "director_agent_id",
            "tmux_session",
            "tmux_window_id",
            "tmux_pane_id",
            "created_at",
        }
        missing = expected_cols - set(cols.keys())
        assert not missing

        # NULL = pending placement before the pane is spawned
        assert cols["tmux_pane_id"]["nullable"] is True

        # NULL marks the root Director's own placement (no parent)
        assert cols["director_agent_id"]["nullable"] is True

        for name in (
            "agent_id",
            "tmux_session",
            "tmux_window_id",
            "created_at",
        ):
            assert cols[name]["nullable"] is False

        indexes = insp.get_indexes("agent_placements")
        idx_names = {idx["name"] for idx in indexes}
        assert "idx_placements_director" in idx_names
    finally:
        engine.dispose()


def test_tasks_table_has_origin_task_id_column(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        cols = {col["name"]: col for col in insp.get_columns("tasks")}

        assert "origin_task_id" in cols

        # Must be nullable because unicast + historical rows store NULL
        assert cols["origin_task_id"]["nullable"] is True
    finally:
        engine.dispose()
