"""Alembic environment — swaps ``sqlite+aiosqlite://`` to sync ``sqlite://``."""

from logging.config import fileConfig

from alembic import context
from alembic.script import ScriptDirectory
from sqlalchemy import engine_from_config, pool
from sqlalchemy.engine.url import make_url

from cafleet.config import settings
from cafleet.db.models import Base

config = context.config

if config.config_file_name is not None:
    fileConfig(config.config_file_name)

target_metadata = Base.metadata


def _sync_url() -> str:
    raw = config.get_main_option("sqlalchemy.url") or settings.database_url
    return str(make_url(raw).set(drivername="sqlite"))


def _use_sequential_revision_id(context, revision, directives) -> None:
    """Mint zero-padded sequential revision ids (0001, 0002, …) instead of
    Alembic's default random hex, so ``alembic revision [--autogenerate]``
    produces ``000N_<slug>.py`` that matches the hand-authored chain and the
    ``test_nine_migration_revisions_exist`` snapshot guard. Generate migrations
    via ``mise //cafleet:makemigration``."""
    if not directives:
        return
    head = ScriptDirectory.from_config(context.config).get_current_head()
    directives[0].rev_id = f"{int(head) + 1:04d}" if head else "0001"


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
            process_revision_directives=_use_sequential_revision_id,
        )
        with context.begin_transaction():
            context.run_migrations()


if context.is_offline_mode():
    run_migrations_offline()
else:
    run_migrations_online()
