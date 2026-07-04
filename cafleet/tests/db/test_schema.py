"""Tests for ``cafleet.db.schema.create_schema`` (the single-baseline schema)."""

import sqlite3
from pathlib import Path

from cafleet import config

BASELINE_TABLES = {
    "fleets",
    "agents",
    "agent_placements",
    "tasks",
    "monitor_config",
    "monitor_runtime",
    "skill_installs",
}

BASELINE_INDEXES = {
    "idx_agents_fleet_status",
    "idx_placements_director",
    "idx_tasks_context_status_ts",
    "idx_tasks_from_agent_status_ts",
}


def _table_names(db_path) -> set[str]:
    """Return the set of user-visible table names in a SQLite file."""
    conn = sqlite3.connect(str(db_path))
    try:
        rows = conn.execute(
            "SELECT name FROM sqlite_master"
            " WHERE type='table' AND name NOT LIKE 'sqlite_%'"
        ).fetchall()
        return {row[0] for row in rows}
    finally:
        conn.close()


def _index_names(db_path) -> set[str]:
    """Return the set of named (non-auto) index names in a SQLite file."""
    conn = sqlite3.connect(str(db_path))
    try:
        rows = conn.execute(
            "SELECT name FROM sqlite_master"
            " WHERE type='index' AND name NOT LIKE 'sqlite_autoindex%'"
        ).fetchall()
        return {row[0] for row in rows}
    finally:
        conn.close()


def test_create_schema_creates_exactly_the_seven_baseline_tables(tmp_path, monkeypatch):
    """A fresh DB gets exactly the seven baseline tables and the four indexes.

    Set equality also proves no legacy schema-version table appears. The DB
    path is placed under a not-yet-existing ``data/`` subdir so the
    parent-dir creation path is exercised.
    """
    db_file = tmp_path / "data" / "cafleet.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    assert not db_file.parent.exists()

    from cafleet.db.schema import create_schema

    returned = create_schema()

    assert isinstance(returned, Path)
    assert returned == db_file
    assert db_file.parent.exists()
    assert db_file.exists()

    assert _table_names(db_file) == BASELINE_TABLES
    assert _index_names(db_file) == BASELINE_INDEXES


def test_create_schema_idempotent(tmp_path, monkeypatch):
    """A second run creates nothing new and alters nothing."""
    db_file = tmp_path / "cafleet.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    from cafleet.db.schema import create_schema

    create_schema()
    tables_after_first = _table_names(db_file)
    indexes_after_first = _index_names(db_file)

    returned = create_schema()

    assert returned == db_file
    assert _table_names(db_file) == tables_after_first == BASELINE_TABLES
    assert _index_names(db_file) == indexes_after_first == BASELINE_INDEXES


def test_create_schema_leaves_preexisting_tables_untouched(tmp_path, monkeypatch):
    """An old Alembic-era DB keeps its orphan ``alembic_version`` table.

    Pre-existing tables are never altered or migrated (``checkfirst`` /
    ``CREATE TABLE IF NOT EXISTS`` semantics); missing baseline tables are
    still created alongside.
    """
    db_file = tmp_path / "cafleet.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    conn = sqlite3.connect(str(db_file))
    try:
        conn.execute(
            "CREATE TABLE alembic_version"
            " (version_num VARCHAR(32) NOT NULL PRIMARY KEY)"
        )
        conn.execute("INSERT INTO alembic_version (version_num) VALUES ('0005')")
        conn.commit()
    finally:
        conn.close()

    from cafleet.db.schema import create_schema

    create_schema()

    assert _table_names(db_file) == BASELINE_TABLES | {"alembic_version"}

    conn = sqlite3.connect(str(db_file))
    try:
        rows = conn.execute("SELECT version_num FROM alembic_version").fetchall()
    finally:
        conn.close()
    assert rows == [("0005",)]
