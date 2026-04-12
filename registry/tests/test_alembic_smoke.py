"""Alembic smoke test — the only test that exercises the real migration script.

The design doc (Testing Strategy → "Trade-off") explains why this file exists:

  "the in-memory fixture uses Base.metadata.create_all, which bypasses
   Alembic. A schema change that is correct in models but missing from a
   migration would slip through this fixture stack. To catch that, a
   separate session-level Alembic smoke test runs against a real tempfile DB."

The migration is run exactly once per pytest session via a session-scoped
fixture. Each test below is a read-only assertion against the resulting
DB file, so amortizing the upgrade across the session keeps the suite fast.

Why a tempfile DB and the sync ``sqlite://`` driver:
  * Alembic's migration runner is synchronous; the async ``aiosqlite``
    driver does not buy anything for migrations and complicates the
    connection lifecycle.
  * SQLite ``:memory:`` databases are per-connection. Alembic opens
    multiple connections during ``upgrade head``; each would see its own
    empty DB, so the migration would silently no-op.

Why ``cfg.set_main_option("sqlalchemy.url", ...)`` instead of monkeypatching
``settings.database_url``:
  * The Alembic ``Config`` object is the canonical place to override the
    DB URL for a one-off migration run. env.py is expected to read
    ``config.get_main_option("sqlalchemy.url")`` and fall back to
    ``settings.database_url`` only when the cfg main option is unset.
  * This keeps the test independent of the user's ``HIKYAKU_DATABASE_URL``
    environment variable and the cached ``config.settings`` singleton.
"""

import importlib.resources

import pytest
from alembic import command
from alembic.config import Config
from sqlalchemy import create_engine, inspect, text


@pytest.fixture(scope="session")
def alembic_upgraded_db(tmp_path_factory):
    """Run ``alembic upgrade head`` once per session against a tempfile DB.

    Returns the path to the migrated SQLite file. Two design choices
    matter here and are worth being explicit about:

    1. The bundled ``alembic.ini`` is located via ``importlib.resources``
       (NOT via a hard-coded path relative to the test file), so this
       fixture works whether ``hikyaku_registry`` is imported from a
       source checkout or from an installed wheel. The
       ``importlib.resources.as_file`` context manager guarantees a
       real filesystem path even when the package data lives inside a
       zipped wheel.

    2. The DB URL is injected via ``cfg.set_main_option("sqlalchemy.url",
       ...)`` rather than by monkeypatching ``config.settings``. This is
       the design-doc-prescribed pattern (Step 3): env.py reads the cfg
       main option first and only falls back to ``settings.database_url``
       when the option is unset. Routing the override through cfg keeps
       the migration's URL resolution local to this fixture and immune to
       Settings-singleton caching surprises.
    """
    tmp_db_path = tmp_path_factory.mktemp("alembic_smoke") / "smoke.db"

    with importlib.resources.as_file(
        importlib.resources.files("hikyaku_registry") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{tmp_db_path}")
        command.upgrade(cfg, "head")

    return tmp_db_path


def test_alembic_upgrade_head_creates_expected_tables(alembic_upgraded_db):
    """``alembic upgrade head`` produces the expected tables.

    The expected set is:

      - ``sessions``           — session namespace, PK = ``session_id``
      - ``agents``             — FK ``session_id`` -> ``sessions.session_id``
      - ``tasks``              — FK ``context_id`` -> ``agents.agent_id``
      - ``agent_placements``   — FK ``agent_id`` -> ``agents.agent_id``
      - ``alembic_version``    — Alembic's own bookkeeping table

    ``api_keys`` must NOT be present — it is dropped by migration
    ``0002_local_simplification``.

    This is a superset assertion (``expected <= tables``), not equality:
    if a future migration introduces an additional table, this test
    should not fail spuriously.

    Schema *shape* drift between ``db/models.py`` and the migration is
    NOT caught here — that's the job of the in-memory model tests in
    ``test_db_models.py``. This test only catches the coarser failure
    mode of "the migration script forgot to create a whole table".
    """
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        tables = set(insp.get_table_names())

        expected = {"sessions", "agents", "tasks", "agent_placements", "alembic_version"}
        missing = expected - tables
        assert not missing, (
            f"alembic upgrade head did not create the expected tables. "
            f"missing: {sorted(missing)}, found: {sorted(tables)}"
        )
        assert "api_keys" not in tables, (
            "api_keys table still exists after upgrade head — "
            "migration 0002_local_simplification should have dropped it"
        )
    finally:
        engine.dispose()


def test_alembic_version_table_records_applied_revision(alembic_upgraded_db):
    """``alembic_version`` contains exactly one row after ``upgrade head``.

    Distinguishes "env.py was wired but no migration script actually
    executed" (zero rows) from "migration successfully applied" (one
    row). Without this assertion, the table-existence check above would
    happily pass even if the migration script were an empty no-op.
    """
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        with engine.connect() as conn:
            result = conn.execute(text("SELECT version_num FROM alembic_version"))
            rows = result.fetchall()
        assert len(rows) == 1, (
            f"alembic_version should contain exactly one row after "
            f"`upgrade head`, got {len(rows)} rows: {rows}"
        )
    finally:
        engine.dispose()


def test_agent_placements_table_created_by_migration(alembic_upgraded_db):
    """Migration 0003 creates the ``agent_placements`` table with expected columns.

    Like the ``origin_task_id`` test below, this is the ONLY test that
    exercises the real Alembic migration for this table — the fast
    in-memory fixture in ``conftest.py`` uses ``Base.metadata.create_all``
    and bypasses Alembic entirely. A ``db/models.py`` update without a
    matching ``0003_add_agent_placements.py`` (or vice versa) slips past
    every other test in the suite and is only caught here.

    Verified schema properties:

      - Table exists
      - All six columns present
      - ``tmux_pane_id`` is nullable (NULL = pending placement)
      - ``idx_placements_director`` index exists on ``director_agent_id``
    """
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)

        # Table exists
        tables = set(insp.get_table_names())
        assert "agent_placements" in tables, (
            f"migration 0003_add_agent_placements did not create the "
            f"agent_placements table. tables found: {sorted(tables)}"
        )

        # All columns present
        cols = {col["name"]: col for col in insp.get_columns("agent_placements")}
        expected_cols = {
            "agent_id",
            "director_agent_id",
            "tmux_session",
            "tmux_window_id",
            "tmux_pane_id",
            "created_at",
        }
        missing = expected_cols - set(cols.keys())
        assert not missing, (
            f"agent_placements table is missing columns: {sorted(missing)}. "
            f"columns found: {sorted(cols)}"
        )

        # tmux_pane_id must be nullable (NULL = pending placement)
        assert cols["tmux_pane_id"]["nullable"] is True, (
            "agent_placements.tmux_pane_id must be nullable — NULL signals "
            "a pending placement before the pane is spawned"
        )

        # All other columns must be NOT NULL
        for name in (
            "agent_id",
            "director_agent_id",
            "tmux_session",
            "tmux_window_id",
            "created_at",
        ):
            assert cols[name]["nullable"] is False, (
                f"agent_placements.{name} should be NOT NULL"
            )

        # Director index exists
        indexes = insp.get_indexes("agent_placements")
        idx_names = {idx["name"] for idx in indexes}
        assert "idx_placements_director" in idx_names, (
            f"expected idx_placements_director index on agent_placements, "
            f"got: {sorted(idx_names)}"
        )
    finally:
        engine.dispose()


def test_tasks_table_has_origin_task_id_column(alembic_upgraded_db):
    """Migration 0002 adds ``origin_task_id`` as a nullable TEXT column on ``tasks``.

    This is the ONLY test that exercises the real Alembic migration
    script — the fast in-memory fixture in ``conftest.py`` uses
    ``Base.metadata.create_all`` and bypasses Alembic entirely. A
    ``db/models.py`` update without a matching ``0002_add_origin_task_id.py``
    (or vice versa) slips past every other test in the suite and is only
    caught here.

    The column MUST be nullable: unicast deliveries and historical rows
    predating the migration carry NULL, and the broadcast-grouping
    predicate documented in design doc 0000013 is
    ``origin_task_id IS NOT NULL``.
    """
    engine = create_engine(f"sqlite:///{alembic_upgraded_db}")
    try:
        insp = inspect(engine)
        cols = {col["name"]: col for col in insp.get_columns("tasks")}

        assert "origin_task_id" in cols, (
            "migration 0002_add_origin_task_id did not add the "
            "origin_task_id column to the tasks table. "
            f"columns found: {sorted(cols)}"
        )
        assert cols["origin_task_id"]["nullable"] is True, (
            "migration 0002 added origin_task_id as NOT NULL — it must "
            "be nullable because unicast + historical rows store NULL"
        )
    finally:
        engine.dispose()
