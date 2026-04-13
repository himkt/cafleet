"""Sync SQLAlchemy engine + sessionmaker singletons for the broker.

The module-level ``event.listens_for(Engine, "connect")`` callback registers
globally on import. Every engine constructed in this process — including
ad-hoc ones built by tests — will issue ``PRAGMA foreign_keys=ON`` and
``PRAGMA busy_timeout=5000`` on every new raw ``sqlite3`` DBAPI connection.
The listener short-circuits for non-SQLite DBAPIs so unrelated engines
(e.g. in third-party libraries) are not perturbed.
"""

import sqlite3

from sqlalchemy import create_engine, event
from sqlalchemy.engine import Engine
from sqlalchemy.engine.url import make_url
from sqlalchemy.orm import Session, sessionmaker

from cafleet.config import settings

_sync_engine: Engine | None = None
_sync_sessionmaker: sessionmaker[Session] | None = None


@event.listens_for(Engine, "connect")
def _enable_sqlite_pragmas(dbapi_connection, _connection_record) -> None:
    if not isinstance(dbapi_connection, sqlite3.Connection):
        return
    cursor = dbapi_connection.cursor()
    cursor.execute("PRAGMA foreign_keys=ON")
    cursor.execute("PRAGMA busy_timeout=5000")
    cursor.close()


def get_sync_engine() -> Engine:
    global _sync_engine
    if _sync_engine is None:
        sync_url = str(make_url(settings.database_url).set(drivername="sqlite"))
        _sync_engine = create_engine(sync_url)
    return _sync_engine


def get_sync_sessionmaker() -> sessionmaker[Session]:
    global _sync_sessionmaker
    if _sync_sessionmaker is None:
        _sync_sessionmaker = sessionmaker(get_sync_engine(), expire_on_commit=False)
    return _sync_sessionmaker
