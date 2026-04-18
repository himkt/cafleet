"""Regression tests for the SQLite PRAGMA connect-listener."""

from sqlalchemy import create_engine, text

import cafleet.db.engine  # noqa: F401 — registers the PRAGMA listener globally


def test_foreign_keys_enabled():
    engine = create_engine("sqlite:///:memory:")
    with engine.connect() as conn:
        assert conn.execute(text("PRAGMA foreign_keys")).scalar() == 1


def test_busy_timeout_set():
    engine = create_engine("sqlite:///:memory:")
    with engine.connect() as conn:
        assert conn.execute(text("PRAGMA busy_timeout")).scalar() == 5000
