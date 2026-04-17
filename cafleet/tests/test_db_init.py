"""Tests for the ``cafleet db init`` CLI command."""

import sqlite3

from click.testing import CliRunner

from cafleet import config


def _table_names(db_path) -> set[str]:
    """Return the set of user-visible table names in a SQLite file."""
    conn = sqlite3.connect(str(db_path))
    try:
        rows = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        ).fetchall()
        return {row[0] for row in rows}
    finally:
        conn.close()


def test_db_init_creates_schema(tmp_path, monkeypatch):
    """Verifies design-doc state #1: DB file does not exist.

    DB path is placed under a not-yet-existing ``data/`` subdir so the
    ``Path.parent.mkdir(parents=True, exist_ok=True)`` path is exercised.
    """
    db_file = tmp_path / "data" / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    assert not db_file.parent.exists()

    from cafleet.cli import cli

    runner = CliRunner()
    result = runner.invoke(cli, ["db", "init"])

    assert result.exit_code == 0, result.output
    assert db_file.parent.exists()
    assert db_file.exists()

    tables = _table_names(db_file)
    expected = {"sessions", "agents", "tasks", "agent_placements", "alembic_version"}
    assert expected <= tables

    assert "applied" in result.output.lower()


def test_db_init_idempotent(tmp_path, monkeypatch):
    """Verifies design-doc idempotency: second run is a no-op at state #3."""
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    from cafleet.cli import cli

    runner = CliRunner()

    first = runner.invoke(cli, ["db", "init"])
    assert first.exit_code == 0, first.output

    tables_after_first = _table_names(db_file)
    expected = {"sessions", "agents", "tasks", "agent_placements", "alembic_version"}
    assert expected <= tables_after_first

    conn = sqlite3.connect(str(db_file))
    try:
        version_after_first = conn.execute(
            "SELECT version_num FROM alembic_version"
        ).fetchall()
    finally:
        conn.close()

    second = runner.invoke(cli, ["db", "init"])
    assert second.exit_code == 0, second.output
    assert "already at head" in second.output.lower()

    tables_after_second = _table_names(db_file)
    assert tables_after_second == tables_after_first

    conn = sqlite3.connect(str(db_file))
    try:
        version_after_second = conn.execute(
            "SELECT version_num FROM alembic_version"
        ).fetchall()
    finally:
        conn.close()
    assert version_after_second == version_after_first


def test_db_init_legacy_errors(tmp_path, monkeypatch):
    """Verifies design-doc state #6: legacy DB (tables, no alembic_version).

    No auto-stamp -- silently stamping would lie about revision and
    could mask schema mismatches at runtime.
    """
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    conn = sqlite3.connect(str(db_file))
    try:
        conn.execute("CREATE TABLE legacy_squat (id INTEGER PRIMARY KEY)")
        conn.commit()
    finally:
        conn.close()

    from cafleet.cli import cli

    runner = CliRunner()
    result = runner.invoke(cli, ["db", "init"])

    assert result.exit_code == 1, result.output
    assert "alembic stamp head" in result.output

    tables = _table_names(db_file)
    assert "legacy_squat" in tables
    assert "alembic_version" not in tables


def test_db_init_ahead_errors(tmp_path, monkeypatch):
    """Verifies design-doc state #5: ahead-of-head revision is refused.

    Uses a fictional ``9999_future_revision`` that is unknown to the
    local Alembic script directory, triggering the ahead-of-head branch.
    """
    db_file = tmp_path / "registry.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    conn = sqlite3.connect(str(db_file))
    try:
        conn.execute(
            "CREATE TABLE alembic_version "
            "(version_num VARCHAR(32) NOT NULL PRIMARY KEY)"
        )
        conn.execute(
            "INSERT INTO alembic_version (version_num) VALUES ('9999_future_revision')"
        )
        conn.commit()
    finally:
        conn.close()

    from cafleet.cli import cli

    runner = CliRunner()
    result = runner.invoke(cli, ["db", "init"])

    assert result.exit_code == 1, result.output
    output_lower = result.output.lower()
    assert (
        "unknown" in output_lower
        or "ahead" in output_lower
        or "9999_future_revision" in result.output
    )

    conn = sqlite3.connect(str(db_file))
    try:
        rows = conn.execute("SELECT version_num FROM alembic_version").fetchall()
    finally:
        conn.close()
    assert rows == [("9999_future_revision",)]
