"""Tests for db/engine.py — engine factory, sessionmaker factory, FK PRAGMA listener.

The design doc requires:
  - `get_engine() -> AsyncEngine` singleton constructed from `settings.database_url`
  - `get_sessionmaker() -> async_sessionmaker[AsyncSession]` singleton bound to the engine
  - `dispose_engine()` for lifespan teardown
  - `event.listens_for(Engine, "connect")` listener that issues `PRAGMA foreign_keys=ON`
    on every new raw DBAPI connection (otherwise SQLite silently ignores FK declarations)

These tests are intentionally self-contained: they do not depend on the new
conftest.py fixture stack (which lands in Step 12). They use ad-hoc engines and
monkeypatch ``settings.database_url`` so no real DB file is ever created.
"""

import pytest
from sqlalchemy.ext.asyncio import (
    AsyncEngine,
    async_sessionmaker,
    create_async_engine,
)

# Importing the engine module is what registers the global FK PRAGMA listener
# (`event.listens_for(Engine, "connect")`). All subsequent engines created in
# this process — including ones created inline by tests — will have FK enabled.
import cafleet.db.engine  # noqa: F401
from cafleet.db.engine import (
    dispose_engine,
    get_engine,
    get_sessionmaker,
)


# ---------------------------------------------------------------------------
# Test isolation: monkeypatch settings.database_url to in-memory and reset
# the singleton before AND after every test in this file.
# ---------------------------------------------------------------------------


@pytest.fixture(autouse=True)
async def _isolate_engine_singleton(monkeypatch):
    """Override settings.database_url + reset the singleton between tests.

    Without the override, ``get_engine()`` would construct an engine pointing at
    the user's real ``~/.local/share/cafleet/registry.db`` and create that file
    on disk. With it, every test runs against a fresh in-memory SQLite DB.
    """
    from cafleet import config

    monkeypatch.setattr(
        config.settings,
        "database_url",
        "sqlite+aiosqlite:///:memory:",
    )
    await dispose_engine()
    yield
    await dispose_engine()


# ---------------------------------------------------------------------------
# FK PRAGMA listener — the central regression test for Step 2.
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_db_engine_fk_pragma():
    """A fresh connection from a new engine has PRAGMA foreign_keys=1.

    This is the regression test explicitly required by the design doc
    (Testing Strategy → "test_db_engine_fk_pragma"). If the engine module's
    ``event.listens_for(Engine, "connect")`` callback is missing, this test
    fails and every FK declared in db/models.py is silently unenforced.
    """
    engine = create_async_engine("sqlite+aiosqlite:///:memory:")
    try:
        async with engine.connect() as conn:
            result = await conn.exec_driver_sql("PRAGMA foreign_keys")
            row = result.fetchone()
            assert row is not None
            assert row[0] == 1
    finally:
        await engine.dispose()


@pytest.mark.asyncio
async def test_fk_pragma_active_on_subsequent_connections():
    """The listener fires on every new connection, not just the first.

    SQLAlchemy may pool DBAPI connections; the PRAGMA listener must run on
    every fresh DBAPI ``connect`` event so that pooled and freshly-established
    connections both have FK enforcement enabled.
    """
    engine = create_async_engine("sqlite+aiosqlite:///:memory:")
    try:
        for _ in range(3):
            async with engine.connect() as conn:
                result = await conn.exec_driver_sql("PRAGMA foreign_keys")
                row = result.fetchone()
                assert row is not None
                assert row[0] == 1
    finally:
        await engine.dispose()


@pytest.mark.asyncio
async def test_fk_pragma_active_on_engine_built_via_factory():
    """The PRAGMA listener also applies to engines returned by ``get_engine()``."""
    engine = get_engine()
    async with engine.connect() as conn:
        result = await conn.exec_driver_sql("PRAGMA foreign_keys")
        row = result.fetchone()
        assert row[0] == 1


# ---------------------------------------------------------------------------
# Engine factory — singleton semantics
# ---------------------------------------------------------------------------


class TestGetEngine:
    """Tests for ``get_engine()`` — singleton AsyncEngine factory."""

    @pytest.mark.asyncio
    async def test_returns_async_engine(self):
        """``get_engine()`` returns a SQLAlchemy AsyncEngine."""
        engine = get_engine()
        assert isinstance(engine, AsyncEngine)

    @pytest.mark.asyncio
    async def test_returns_same_instance_on_repeated_calls(self):
        """Successive calls return the same engine instance (singleton)."""
        e1 = get_engine()
        e2 = get_engine()
        assert e1 is e2

    @pytest.mark.asyncio
    async def test_uses_settings_database_url(self, monkeypatch):
        """The engine is constructed from ``settings.database_url`` at first call."""
        from cafleet import config

        monkeypatch.setattr(
            config.settings,
            "database_url",
            "sqlite+aiosqlite:///:memory:",
        )
        # Reset so first call below picks up the override.
        await dispose_engine()

        engine = get_engine()
        # The URL on the engine should reflect the settings value.
        assert str(engine.url) == "sqlite+aiosqlite:///:memory:"


# ---------------------------------------------------------------------------
# Sessionmaker factory — singleton semantics + binding
# ---------------------------------------------------------------------------


class TestGetSessionmaker:
    """Tests for ``get_sessionmaker()`` — singleton async_sessionmaker factory."""

    @pytest.mark.asyncio
    async def test_returns_async_sessionmaker(self):
        """``get_sessionmaker()`` returns an ``async_sessionmaker``."""
        sm = get_sessionmaker()
        assert isinstance(sm, async_sessionmaker)

    @pytest.mark.asyncio
    async def test_returns_same_instance_on_repeated_calls(self):
        """Successive calls return the same sessionmaker (singleton)."""
        sm1 = get_sessionmaker()
        sm2 = get_sessionmaker()
        assert sm1 is sm2

    @pytest.mark.asyncio
    async def test_sessionmaker_yields_usable_session(self):
        """A session built from the sessionmaker can be opened and closed."""
        sm = get_sessionmaker()
        async with sm() as session:
            # A trivial driver-level statement to confirm the session is wired
            # to a working engine. Use SELECT 1 (works on any backend).
            result = await session.execute(_select_one())
            assert result.scalar_one() == 1


def _select_one():
    """Build a ``SELECT 1`` Core statement (avoids importing ``text`` at top)."""
    from sqlalchemy import text

    return text("SELECT 1")


# ---------------------------------------------------------------------------
# dispose_engine — teardown semantics
# ---------------------------------------------------------------------------


class TestDisposeEngine:
    """Tests for ``dispose_engine()`` — engine teardown for app lifespan."""

    @pytest.mark.asyncio
    async def test_clears_singleton_so_next_call_returns_fresh_instance(self):
        """After dispose, a subsequent get_engine call returns a new engine."""
        e1 = get_engine()
        await dispose_engine()
        e2 = get_engine()
        assert e1 is not e2

    @pytest.mark.asyncio
    async def test_idempotent_when_no_engine_exists(self):
        """Calling dispose with no engine constructed is a no-op (no exception)."""
        # The autouse fixture already disposed before yielding to this test.
        await dispose_engine()
        await dispose_engine()  # second consecutive call must also be safe

    @pytest.mark.asyncio
    async def test_also_clears_sessionmaker_singleton(self):
        """After dispose, a subsequent get_sessionmaker call returns a new instance.

        The sessionmaker is bound to the engine; after the engine is disposed,
        the cached sessionmaker would point at a defunct engine, so it must be
        cleared too.
        """
        sm1 = get_sessionmaker()
        await dispose_engine()
        sm2 = get_sessionmaker()
        assert sm1 is not sm2
