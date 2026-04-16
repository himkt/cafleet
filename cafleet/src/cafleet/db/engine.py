"""Sync SQLAlchemy engine + sessionmaker singletons.

Importing this module registers a global ``Engine.connect`` listener that
applies ``PRAGMA foreign_keys=ON`` and ``busy_timeout=5000`` on every new
SQLite DBAPI connection — including ad-hoc engines built by tests. The
listener short-circuits on non-SQLite DBAPIs.
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
        _sync_engine = create_engine(
            sync_url, connect_args={"check_same_thread": False}
        )
    return _sync_engine


def get_sync_sessionmaker() -> sessionmaker[Session]:
    global _sync_sessionmaker
    if _sync_sessionmaker is None:
        _sync_sessionmaker = sessionmaker(get_sync_engine(), expire_on_commit=False)
    return _sync_sessionmaker
