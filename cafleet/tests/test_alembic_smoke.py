"""Alembic smoke test — runs real migrations against a tempfile DB.

Other tests use Base.metadata.create_all and bypass Alembic entirely,
so this is the only place that catches migration-vs-model drift.
"""

import importlib.resources

import pytest
from alembic import command
from alembic.config import Config
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
            "sessions",
            "agents",
            "tasks",
            "agent_placements",
            "alembic_version",
        }
        missing = expected - tables
        assert not missing, (
            f"alembic upgrade head did not create the expected tables. "
            f"missing: {sorted(missing)}, found: {sorted(tables)}"
        )
        assert "api_keys" not in tables, (
            "api_keys table still exists after upgrade head — "
            "migration 0002_local_simplification should have dropped it"
        )
    finally:
        engine.dispose()


def test_alembic_version_table_records_applied_revision(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        with engine.connect() as conn:
            result = conn.execute(text("SELECT version_num FROM alembic_version"))
            rows = result.fetchall()
        assert len(rows) == 1, (
            f"alembic_version should contain exactly one row after "
            f"`upgrade head`, got {len(rows)} rows: {rows}"
        )
    finally:
        engine.dispose()


def test_agent_placements_table_created_by_migration(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)

        # Table exists
        tables = set(insp.get_table_names())
        assert "agent_placements" in tables, (
            f"migration 0003_add_agent_placements did not create the "
            f"agent_placements table. tables found: {sorted(tables)}"
        )

        # All columns present
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
        assert not missing, (
            f"agent_placements table is missing columns: {sorted(missing)}. "
            f"columns found: {sorted(cols)}"
        )

        # tmux_pane_id must be nullable (NULL = pending placement)
        assert cols["tmux_pane_id"]["nullable"] is True, (
            "agent_placements.tmux_pane_id must be nullable — NULL signals "
            "a pending placement before the pane is spawned"
        )

        # director_agent_id must be nullable after migration 0007 — NULL
        # signals "this placement is for a root Director (no parent)"
        # per design doc 0000026.
        assert cols["director_agent_id"]["nullable"] is True, (
            "agent_placements.director_agent_id must be nullable after "
            "migration 0007 — NULL marks the root Director's own placement"
        )

        # All other columns must be NOT NULL
        for name in (
            "agent_id",
            "tmux_session",
            "tmux_window_id",
            "created_at",
        ):
            assert cols[name]["nullable"] is False, (
                f"agent_placements.{name} should be NOT NULL"
            )

        # Director index exists
        indexes = insp.get_indexes("agent_placements")
        idx_names = {idx["name"] for idx in indexes}
        assert "idx_placements_director" in idx_names, (
            f"expected idx_placements_director index on agent_placements, "
            f"got: {sorted(idx_names)}"
        )
    finally:
        engine.dispose()


def test_tasks_table_has_origin_task_id_column(alembic_upgraded_db):
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        cols = {col["name"]: col for col in insp.get_columns("tasks")}

        assert "origin_task_id" in cols, (
            "migration 0002_add_origin_task_id did not add the "
            "origin_task_id column to the tasks table. "
            f"columns found: {sorted(cols)}"
        )
        assert cols["origin_task_id"]["nullable"] is True, (
            "migration 0002 added origin_task_id as NOT NULL — it must "
            "be nullable because unicast + historical rows store NULL"
        )
    finally:
        engine.dispose()
