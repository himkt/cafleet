"""Shared test helpers (alembic / registry setup / time) for the cafleet test suite."""

import importlib.metadata
import sqlite3
from datetime import UTC, datetime

from sqlalchemy.engine.url import make_url

from cafleet.config import settings
from cafleet.db.init import run_db_init


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()


def _init_registry() -> None:
    """Migrate the registry to head and record a current skills install.

    The seeded ``skill_installs`` row lets fleet-scoped CLI commands pass the
    stale-skills version guard.
    """
    run_db_init()
    db_file = make_url(settings.database_url).database
    conn = sqlite3.connect(str(db_file))
    try:
        conn.execute(
            "INSERT OR REPLACE INTO skill_installs"
            " (coding_agent, cafleet_version, installed_at) VALUES (?, ?, ?)",
            ("claude", importlib.metadata.version("cafleet"), _now_iso()),
        )
        conn.commit()
    finally:
        conn.close()
