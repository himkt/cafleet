"""Tests for ``run_db_init`` and the ``cafleet setup db`` CLI command."""

import sqlite3
from pathlib import Path

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


def test_default_database_url_points_at_cafleet_v5_db():
    """The default registry file is ``~/.local/share/cafleet/cafleet_v5.db``."""
    url = config._default_database_url()
    expected = Path("~/.local/share/cafleet/cafleet_v5.db").expanduser()
    assert url == f"sqlite:///{expected}"


def test_setup_db_creates_schema(tmp_path, monkeypatch):
    """A fresh ``setup db`` creates the DB file and migrates it to head.

    DB path is placed under a not-yet-existing ``data/`` subdir so the
    ``Path.parent.mkdir(parents=True, exist_ok=True)`` path is exercised.
    """
    db_file = tmp_path / "data" / "cafleet.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    assert not db_file.parent.exists()

    from cafleet.cli import cli

    runner = CliRunner()
    result = runner.invoke(cli, ["setup", "db"])

    assert result.exit_code == 0, result.output
    assert db_file.parent.exists()
    assert db_file.exists()

    tables = _table_names(db_file)
    expected = {
        "fleets",
        "members",
        "messages",
        "member_placements",
        "asset_installs",
        "alembic_version",
    }
    assert expected <= tables

    assert "applied" in result.output.lower()


def test_setup_db_idempotent(tmp_path, monkeypatch):
    """A second ``setup db`` run is a no-op reporting ``Already at head``."""
    db_file = tmp_path / "cafleet.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    from cafleet.cli import cli

    runner = CliRunner()

    first = runner.invoke(cli, ["setup", "db"])
    assert first.exit_code == 0, first.output

    tables_after_first = _table_names(db_file)
    expected = {"fleets", "members", "messages", "member_placements", "alembic_version"}
    assert expected <= tables_after_first

    conn = sqlite3.connect(str(db_file))
    try:
        version_after_first = conn.execute(
            "SELECT version_num FROM alembic_version"
        ).fetchall()
    finally:
        conn.close()

    second = runner.invoke(cli, ["setup", "db"])
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


def test_setup_db_errors_on_unversioned_db(tmp_path, monkeypatch):
    """A database with tables but no ``alembic_version`` is rejected, not
    auto-stamped -- silently stamping would lie about the revision and could
    mask schema mismatches at runtime.
    """
    db_file = tmp_path / "cafleet.db"
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
    result = runner.invoke(cli, ["setup", "db"])

    assert result.exit_code == 1, result.output
    assert "alembic stamp head" in result.output

    tables = _table_names(db_file)
    assert "legacy_squat" in tables
    assert "alembic_version" not in tables


def test_setup_db_ahead_errors(tmp_path, monkeypatch):
    """An ahead-of-head revision unknown to the local script directory is refused.

    Uses a fictional ``9999_future_revision`` that is unknown to the
    local Alembic script directory, triggering the ahead-of-head branch.
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
    result = runner.invoke(cli, ["setup", "db"])

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


def test_run_db_init_creates_schema_at_head(tmp_path, monkeypatch, capsys):
    """``run_db_init()`` called directly creates the schema at head.

    Exercises the reusable helper the way ``cafleet setup``'s database half
    invokes it -- no CLI runner, no skills mocking.
    """
    db_file = tmp_path / "data" / "cafleet.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    assert not db_file.parent.exists()

    from cafleet.db.init import run_db_init

    run_db_init()

    assert db_file.parent.exists()
    assert db_file.exists()

    tables = _table_names(db_file)
    expected = {
        "fleets",
        "members",
        "messages",
        "member_placements",
        "asset_installs",
        "alembic_version",
    }
    assert expected <= tables

    assert "applied" in capsys.readouterr().out.lower()


def test_run_db_init_idempotent_reports_already_at_head(tmp_path, monkeypatch, capsys):
    """A second direct ``run_db_init()`` is a no-op reporting ``Already at head``."""
    db_file = tmp_path / "cafleet.db"
    monkeypatch.setattr(
        config.settings,
        "database_url",
        f"sqlite+aiosqlite:///{db_file}",
    )

    from cafleet.db.init import run_db_init

    run_db_init()
    capsys.readouterr()

    conn = sqlite3.connect(str(db_file))
    try:
        version_after_first = conn.execute(
            "SELECT version_num FROM alembic_version"
        ).fetchall()
    finally:
        conn.close()

    run_db_init()

    assert "already at head" in capsys.readouterr().out.lower()

    conn = sqlite3.connect(str(db_file))
    try:
        version_after_second = conn.execute(
            "SELECT version_num FROM alembic_version"
        ).fetchall()
    finally:
        conn.close()
    assert version_after_second == version_after_first


def test_setup_db_delegates_to_run_db_init(monkeypatch):
    """``cafleet setup db`` is a thin wrapper that calls ``run_db_init()``."""
    calls = []

    from cafleet.cli import setup as setup_module

    monkeypatch.setattr(setup_module, "run_db_init", lambda: calls.append(True))

    from cafleet.cli import cli

    runner = CliRunner()
    result = runner.invoke(cli, ["setup", "db"])

    assert result.exit_code == 0, result.output
    assert calls == [True]
