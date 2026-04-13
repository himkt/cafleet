"""Regression tests for SQLite PRAGMA settings.

Verifies that ``_enable_sqlite_pragmas`` fires on every new connection
and sets both ``foreign_keys=ON`` and ``busy_timeout=5000``.
"""

from sqlalchemy import create_engine, text


def test_foreign_keys_enabled():
    """PRAGMA foreign_keys should be ON for every new connection."""
    # Importing engine module registers the global event listener
    import cafleet.db.engine  # noqa: F401

    engine = create_engine("sqlite:///:memory:")
    with engine.connect() as conn:
        result = conn.execute(text("PRAGMA foreign_keys")).scalar()
        assert result == 1


def test_busy_timeout_set():
    """PRAGMA busy_timeout should be 5000 for every new connection."""
    import cafleet.db.engine  # noqa: F401

    engine = create_engine("sqlite:///:memory:")
    with engine.connect() as conn:
        result = conn.execute(text("PRAGMA busy_timeout")).scalar()
        assert result == 5000
