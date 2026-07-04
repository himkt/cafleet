"""Single-baseline schema creation; see ``docs/spec/data-model.md``."""

from pathlib import Path

from sqlalchemy import create_engine
from sqlalchemy.engine.url import make_url

from cafleet.config import settings
from cafleet.db.models import Base


def create_schema() -> Path:
    """Create the single-baseline schema; returns the DB file path."""
    sync_url = str(make_url(settings.database_url).set(drivername="sqlite"))
    db_file_str = make_url(sync_url).database
    if not db_file_str:
        raise ValueError("database URL has no file path")
    db_file = Path(db_file_str)

    db_file.parent.mkdir(parents=True, exist_ok=True)

    engine = create_engine(sync_url)
    try:
        Base.metadata.create_all(engine)
    finally:
        engine.dispose()
    return db_file
