"""Alembic smoke test — the only test that exercises the real migration script.

The design doc (Testing Strategy → "Trade-off") explains why this file exists:

  "the in-memory fixture uses Base.metadata.create_all, which bypasses
   Alembic. A schema change that is correct in models but missing from a
   migration would slip through this fixture stack. To catch that, a
   separate session-level Alembic smoke test runs against a real tempfile DB."

This file contains exactly one test. It runs `alembic upgrade head` against
a freshly-created tempfile SQLite DB (NOT in-memory — Alembic + aiosqlite
do not play well with `:memory:` because each connection sees its own
empty database) and asserts that the four expected tables exist:
api_keys, agents, tasks, alembic_version.

If db/models.py grows a column without a matching migration, this smoke
test will pass (since `alembic_version` and the four tables still exist)
but other schema-shape tests in test_db_models.py will diverge from the
migration output, surfacing the drift.
"""

from importlib.resources import files

import pytest
from alembic import command
from alembic.config import Config
from sqlalchemy import create_engine, inspect, text


def test_alembic_upgrade_head_creates_expected_schema(tmp_path):
    """`alembic upgrade head` against a fresh tempfile DB produces the four expected tables.

    Verifies the design doc bullet (Step 3): "Add test_alembic_smoke.py
    that runs `command.upgrade(cfg, 'head')` against a tempfile DB and
    asserts the four expected tables exist".

    The override flow:
      1. Build a tempfile path inside pytest's per-test ``tmp_path``.
      2. Monkeypatch ``settings.database_url`` so env.py picks up the
         tempfile URL when Alembic loads it. env.py is the sole consumer
         of ``settings.database_url`` during a migration run; it does
         ``make_url(settings.database_url).set(drivername='sqlite')`` at
         env.py module load time, and Alembic loads env.py freshly on
         every ``command.upgrade`` invocation, so the monkeypatch is
         visible to the migration even though env.py is module-level code.
      3. Build an Alembic ``Config`` from the bundled ``alembic.ini`` via
         ``importlib.resources`` so the test works whether the package is
         imported from a source checkout or from an installed wheel.
      4. Run ``command.upgrade(cfg, 'head')``.
      5. Open a fresh sync engine to the tempfile and assert that
         ``inspect(engine).get_table_names()`` is a superset of the four
         expected tables.
      6. Assert ``alembic_version`` contains exactly one row — that's the
         only way to distinguish "env.py wired but no migration applied"
         from "migration successfully applied".
    """
    db_file = tmp_path / "smoke.db"

    # env.py reads settings.database_url at module load time. Patch the
    # attribute on the existing singleton (not rebind config.settings)
    # so that env.py's `from hikyaku_registry.config import settings`
    # binding sees the new value via attribute lookup.
    with pytest.MonkeyPatch.context() as mp:
        from hikyaku_registry import config

        mp.setattr(
            config.settings,
            "database_url",
            f"sqlite+aiosqlite:///{db_file}",
        )

        # Locate the bundled alembic.ini via importlib.resources so the
        # test works in both editable-install and built-wheel layouts.
        ini_path = files("hikyaku_registry") / "alembic.ini"
        cfg = Config(str(ini_path))

        command.upgrade(cfg, "head")

    # Inspect the resulting DB file with a fresh sync engine. Use the
    # plain `sqlite://` driver (not `sqlite+aiosqlite://`) so the inspector
    # runs synchronously without needing an event loop.
    engine = create_engine(f"sqlite:///{db_file}")
    try:
        insp = inspect(engine)
        tables = set(insp.get_table_names())

        expected = {"api_keys", "agents", "tasks", "alembic_version"}
        missing = expected - tables
        assert not missing, (
            f"alembic upgrade head did not create the expected tables. "
            f"missing: {sorted(missing)}, found: {sorted(tables)}"
        )

        # An empty alembic_version table would indicate that env.py was
        # configured but no migration script actually executed.
        with engine.connect() as conn:
            result = conn.execute(text("SELECT version_num FROM alembic_version"))
            rows = result.fetchall()
        assert len(rows) == 1, (
            f"alembic_version should contain exactly one row after "
            f"`upgrade head`, got {len(rows)} rows: {rows}"
        )
    finally:
        engine.dispose()
