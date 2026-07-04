"""Shared test helpers (registry setup / time) for the cafleet test suite."""

import importlib.metadata
import sqlite3
from datetime import UTC, datetime

from cafleet.db.schema import create_schema


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()


def _init_registry() -> None:
    """Create the baseline schema and record a current skills install.

    The seeded ``skill_installs`` row lets fleet-scoped CLI commands pass the
    stale-skills version guard.
    """
    db_file = create_schema()
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
