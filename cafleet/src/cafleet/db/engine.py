"""Async SQLAlchemy engine + sessionmaker singletons for the registry.

The module-level ``event.listens_for(Engine, "connect")`` callback registers
globally on import. Every engine constructed in this process — including
ad-hoc ones built by tests — will issue ``PRAGMA foreign_keys=ON`` on every
new raw ``sqlite3`` DBAPI connection. SQLite silently ignores foreign-key
declarations unless this PRAGMA is set on the connection that performs the
write. The listener short-circuits for non-SQLite DBAPIs so unrelated
engines (e.g. in third-party libraries) are not perturbed.
"""

import sqlite3

from sqlalchemy import event
from sqlalchemy.engine import Engine
from sqlalchemy.ext.asyncio import (
    AsyncEngine,
    AsyncSession,
    async_sessionmaker,
    create_async_engine,
)

from hikyaku.config import settings

_engine: AsyncEngine | None = None
_sessionmaker: async_sessionmaker[AsyncSession] | None = None


@event.listens_for(Engine, "connect")
def _enable_sqlite_foreign_keys(dbapi_connection, _connection_record) -> None:
    if not isinstance(dbapi_connection, sqlite3.Connection):
        return
    cursor = dbapi_connection.cursor()
    cursor.execute("PRAGMA foreign_keys=ON")
    cursor.close()


def get_engine() -> AsyncEngine:
    global _engine
    if _engine is None:
        _engine = create_async_engine(settings.database_url)
    return _engine


def get_sessionmaker() -> async_sessionmaker[AsyncSession]:
    global _sessionmaker
    if _sessionmaker is None:
        _sessionmaker = async_sessionmaker(get_engine(), expire_on_commit=False)
    return _sessionmaker


async def dispose_engine() -> None:
    global _engine, _sessionmaker
    if _engine is not None:
        await _engine.dispose()
    _engine = None
    _sessionmaker = None
