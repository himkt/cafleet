"""Tests for the stale-assets version guard on the fleet-scoped groups.

``ensure_assets_current()`` runs at the top of the ``fleet`` / ``member`` /
``message`` / ``monitor`` group callbacks — before any subcommand body — and
hard-errors when no assets install is recorded or when any recorded version
differs from the runtime CLI version. ``setup``, ``doctor``, and ``server``
are exempt.
"""

import importlib.metadata
import sqlite3

import pytest
import uvicorn
from click.testing import CliRunner

from cafleet import config
from cafleet.cli import cli
from cafleet.multiplexer import MultiplexerContext as DirectorContext
from cafleet.multiplexer.tmux import TmuxMultiplexer

RUNTIME_VERSION = importlib.metadata.version("cafleet")

NO_INSTALL_ERROR = "no assets install is recorded; run 'cafleet setup' first"
STALE_REPAIR = "run 'cafleet setup' to reinstall"

FLEET_SCOPED_INVOCATIONS = [
    pytest.param(["fleet", "list"], id="fleet"),
    pytest.param(["member", "list", "--fleet-id", "1"], id="member"),
    pytest.param(
        ["message", "poll", "--fleet-id", "1", "--member-id", "1"], id="message"
    ),
    pytest.param(["monitor", "status", "--fleet-id", "1"], id="monitor"),
]


@pytest.fixture(autouse=True)
def registry_db(tmp_path, monkeypatch):
    """Redirect the registry at a temp SQLite so no test touches the real DB."""
    db_path = tmp_path / "registry" / "cafleet.db"
    monkeypatch.setattr(
        config.settings, "database_url", f"sqlite+aiosqlite:///{db_path}"
    )
    return db_path


def _init_schema():
    from cafleet.db.init import run_db_init

    run_db_init()


def _seed_install(
    db_path,
    coding_agent,
    cafleet_version,
    installed_at="2026-07-04T00:12:09.123456+00:00",
):
    conn = sqlite3.connect(str(db_path))
    try:
        conn.execute(
            "INSERT OR REPLACE INTO asset_installs"
            " (coding_agent, cafleet_version, installed_at) VALUES (?, ?, ?)",
            (coding_agent, cafleet_version, installed_at),
        )
        conn.commit()
    finally:
        conn.close()


def _run(args):
    return CliRunner().invoke(cli, args)


# --------------------------------------------------------------------------- #
# Missing install: DB file / table / rows absent                               #
# --------------------------------------------------------------------------- #


@pytest.mark.parametrize("argv", FLEET_SCOPED_INVOCATIONS)
def test_guard_blocks_every_fleet_scoped_group_when_never_setup(argv, registry_db):
    """With no DB file at all, all four fleet-scoped groups hard-error."""
    result = _run(argv)

    assert result.exit_code == 1, result.output
    assert NO_INSTALL_ERROR in result.output


def test_guard_blocks_on_empty_table(registry_db):
    """A created schema with zero recorded rows is 'setup never ran'."""
    _init_schema()

    result = _run(["fleet", "list"])

    assert result.exit_code == 1, result.output
    assert NO_INSTALL_ERROR in result.output


# --------------------------------------------------------------------------- #
# Stale rows                                                                   #
# --------------------------------------------------------------------------- #


def test_guard_stale_row_error_string(registry_db):
    """A single stale row produces the full two-half error message."""
    _init_schema()
    _seed_install(registry_db, "claude", "0.0.1")

    result = _run(["fleet", "list"])

    assert result.exit_code == 1, result.output
    assert (
        f"stale assets detected (claude=0.0.1; CLI {RUNTIME_VERSION}); {STALE_REPAIR}"
    ) in result.output


def test_guard_stale_lists_agents_ascending_and_skips_current(registry_db):
    """Stale agents are listed in ascending order; current rows are not listed."""
    _init_schema()
    _seed_install(registry_db, "codex", "0.0.2")
    _seed_install(registry_db, "claude", "0.0.1")
    _seed_install(registry_db, "opencode", RUNTIME_VERSION)

    result = _run(["fleet", "list"])

    assert result.exit_code == 1, result.output
    assert (
        f"stale assets detected (claude=0.0.1, codex=0.0.2; CLI {RUNTIME_VERSION}); "
        f"{STALE_REPAIR}"
    ) in result.output
    assert "opencode=" not in result.output


def test_guard_downgrade_is_also_stale(registry_db):
    """Mismatch is simple string inequality — a downgrade also triggers."""
    _init_schema()
    _seed_install(registry_db, "claude", "999999.0.0")

    result = _run(["fleet", "list"])

    assert result.exit_code == 1, result.output
    assert "stale assets detected" in result.output
    assert STALE_REPAIR in result.output


# --------------------------------------------------------------------------- #
# Matching rows pass                                                           #
# --------------------------------------------------------------------------- #


def test_guard_matching_rows_pass(registry_db):
    """Rows matching the runtime version let the subcommand run normally."""
    _init_schema()
    _seed_install(registry_db, "claude", RUNTIME_VERSION)

    result = _run(["fleet", "list"])

    assert result.exit_code == 0, result.output
    assert "No fleets found." in result.output


def test_guard_unchecked_homes_do_not_trigger(registry_db):
    """Homes with no row (agent never installed) are not checked."""
    _init_schema()
    _seed_install(registry_db, "opencode", RUNTIME_VERSION)
    # claude / codex never installed: no rows, no complaint

    result = _run(["fleet", "list"])

    assert result.exit_code == 0, result.output


# --------------------------------------------------------------------------- #
# Exempt surfaces: setup, doctor, server                                       #
# --------------------------------------------------------------------------- #


def test_setup_db_is_exempt(registry_db):
    """The repair command must remain runnable with no install recorded."""
    result = _run(["setup", "db"])

    assert result.exit_code == 0, result.output


def test_doctor_is_exempt(registry_db, monkeypatch):
    """``doctor`` reports instead of blocking."""
    monkeypatch.setenv("TMUX", "/tmp/tmux-1000/default,12345,0")
    monkeypatch.setenv("TMUX_PANE", "%0")
    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", lambda self: None)
    monkeypatch.setattr(
        TmuxMultiplexer,
        "context_discovery",
        lambda self: DirectorContext(session="main", window_id="@3", pane_id="%0"),
    )

    result = _run(["doctor"])

    assert result.exit_code == 0, result.output


def test_server_is_exempt(registry_db, monkeypatch):
    """``server`` is human-facing and not fleet-scoped; it starts unguarded."""
    monkeypatch.setattr(uvicorn, "run", lambda *args, **kwargs: None)

    result = _run(["server"])

    assert result.exit_code == 0, result.output


# --------------------------------------------------------------------------- #
# Help contract (recorded in cli-options.md / SPEC)                            #
# --------------------------------------------------------------------------- #


def test_group_help_prints_under_stale_install(registry_db):
    """Group-level help is parsed eagerly and always works."""
    _init_schema()
    _seed_install(registry_db, "claude", "0.0.1")

    result = _run(["fleet", "--help"])

    assert result.exit_code == 0, result.output
    assert "Usage:" in result.output
    assert "stale assets detected" not in result.output


def test_subcommand_help_errors_under_stale_install(registry_db):
    """Subcommand help runs the group callback first, so the guard errors."""
    _init_schema()
    _seed_install(registry_db, "claude", "0.0.1")

    result = _run(["fleet", "create", "--help"])

    assert result.exit_code == 1, result.output
    assert "stale assets detected" in result.output


def test_subcommand_help_errors_when_never_setup(registry_db):
    result = _run(["fleet", "create", "--help"])

    assert result.exit_code == 1, result.output
    assert NO_INSTALL_ERROR in result.output
