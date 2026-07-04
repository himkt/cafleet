"""CLI-level fixtures: every CLI test runs against a seeded temp registry."""

import pytest

from cafleet import config
from tests._helpers import _init_registry


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
