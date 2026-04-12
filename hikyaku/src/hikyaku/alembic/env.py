"""Alembic environment — sync driver against Base metadata.

The application runtime uses ``sqlite+aiosqlite://...`` for async I/O, but
Alembic migrations run synchronously, so we swap the driver to plain
``sqlite://`` via ``make_url(...).set(drivername='sqlite')``.

``~`` expansion is owned by ``config.py`` (constructed at settings load
time), so ``settings.database_url`` is already absolute by the time env.py
reads it.
"""

from logging.config import fileConfig

from alembic import context
from sqlalchemy import engine_from_config, pool
from sqlalchemy.engine.url import make_url

from hikyaku.config import settings
from hikyaku.db.models import Base

config = context.config

if config.config_file_name is not None:
    fileConfig(config.config_file_name)

target_metadata = Base.metadata


def _sync_url() -> str:
    cfg_url = config.get_main_option("sqlalchemy.url")
    raw = cfg_url if cfg_url else settings.database_url
    return str(make_url(raw).set(drivername="sqlite"))


def run_migrations_offline() -> None:
    context.configure(
        url=_sync_url(),
        target_metadata=target_metadata,
        literal_binds=True,
        dialect_opts={"paramstyle": "named"},
        render_as_batch=True,
    )
    with context.begin_transaction():
        context.run_migrations()


def run_migrations_online() -> None:
    configuration = config.get_section(config.config_ini_section) or {}
    configuration["sqlalchemy.url"] = _sync_url()
    connectable = engine_from_config(
        configuration,
        prefix="sqlalchemy.",
        poolclass=pool.NullPool,
    )
    with connectable.connect() as connection:
        context.configure(
            connection=connection,
            target_metadata=target_metadata,
            render_as_batch=True,
        )
        with context.begin_transaction():
            context.run_migrations()


if context.is_offline_mode():
    run_migrations_offline()
else:
    run_migrations_online()
