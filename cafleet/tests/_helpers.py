"""Shared test helpers (alembic / time) for the cafleet test suite."""

import importlib.resources
from datetime import UTC, datetime

from alembic.config import Config


def _make_alembic_cfg(db_path) -> Config:
    with importlib.resources.as_file(
        importlib.resources.files("cafleet") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", f"sqlite:///{db_path}")
        return cfg


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()
