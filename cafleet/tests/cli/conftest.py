"""CLI-level fixtures: every CLI test runs against a seeded temp registry."""

import json

import pytest
from click.testing import CliRunner

from cafleet import config
from cafleet.cli import cli
from cafleet.multiplexer import MultiplexerContext as DirectorContext
from tests._helpers import _init_registry

_FAKE_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")


@pytest.fixture(autouse=True)
def _cli_registry(tmp_path, monkeypatch, _reset_engine_singletons):
    """Redirect the registry at a seeded temp SQLite for every CLI test.

    Fleet-scoped commands run the stale-skills guard against the engine built
    from ``settings.database_url``; without this redirect a CLI test that never
    patches the broker session would read the operator's real registry (and
    fail the guard). Per-file fixtures that re-redirect ``database_url`` run
    after this one and win, so unseeded/empty-DB scenarios stay expressible.
    """
    db_path = tmp_path / "cli-registry" / "cafleet.db"
    monkeypatch.setattr(
        config.settings, "database_url", f"sqlite+aiosqlite:///{db_path}"
    )
    _init_registry()
    return db_path


@pytest.fixture
def _mock_tmux_for_fleet_create(monkeypatch):
    """Opt-in: let CliRunner-driven ``fleet create`` succeed without a real tmux pane."""
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.ensure_available",
        lambda self: None,
    )
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.context_discovery",
        lambda self: _FAKE_DIRECTOR_CTX,
    )


@pytest.fixture
def make_bootstrapped_fleet(_mock_tmux_for_fleet_create):
    """Factory: run ``fleet create --json`` and return ``(runner, data)``.

    The autouse ``_cli_registry`` already seeds a fresh temp DB, so callers do
    not redirect ``database_url`` themselves. The tmux stub is pulled in as a
    dependency so ``fleet create`` succeeds without a real pane.
    """

    def _make() -> tuple[CliRunner, dict]:
        runner = CliRunner()
        result = runner.invoke(
            cli,
            [
                "fleet",
                "create",
                "--name",
                "test-fleet",
                "--coding-agent",
                "claude",
                "--json",
            ],
        )
        assert result.exit_code == 0, result.output
        return runner, json.loads(result.output)

    return _make
