"""Shared test fixtures for hikyaku-registry tests.

A function-scoped in-memory aiosqlite engine + SQLAlchemy
``Base.metadata.create_all`` is the canonical fast-path described in
the design doc's Testing Strategy section: each test gets a clean DB
with no Alembic overhead, and FK enforcement is guaranteed by a
module-level ``connect`` listener that mirrors ``db/engine.py``.

The fixture stack here is intentionally minimal — only what Step 5
needs (RegistryStore). The ``task_store`` fixture lands alongside
Step 6.
"""

import pytest
from sqlalchemy import event
from sqlalchemy.engine import Engine
from sqlalchemy.ext.asyncio import (
    AsyncEngine,
    AsyncSession,
    async_sessionmaker,
    create_async_engine,
)

from hikyaku_registry.db.models import Base
from hikyaku_registry.registry_store import RegistryStore
from hikyaku_registry.task_store import TaskStore


@event.listens_for(Engine, "connect")
def _enable_fk_pragma(dbapi_conn, _record):
    cursor = dbapi_conn.cursor()
    cursor.execute("PRAGMA foreign_keys=ON")
    cursor.close()


@pytest.fixture
async def db_engine() -> AsyncEngine:
    engine = create_async_engine("sqlite+aiosqlite:///:memory:")
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
    yield engine
    await engine.dispose()


@pytest.fixture
async def db_sessionmaker(
    db_engine: AsyncEngine,
) -> async_sessionmaker[AsyncSession]:
    return async_sessionmaker(db_engine, expire_on_commit=False)


@pytest.fixture
async def db_session(
    db_sessionmaker: async_sessionmaker[AsyncSession],
) -> AsyncSession:
    async with db_sessionmaker() as session:
        yield session


@pytest.fixture
async def store(
    db_sessionmaker: async_sessionmaker[AsyncSession],
) -> RegistryStore:
    return RegistryStore(db_sessionmaker)


@pytest.fixture
async def task_store(
    db_sessionmaker: async_sessionmaker[AsyncSession],
) -> TaskStore:
    return TaskStore(db_sessionmaker)
