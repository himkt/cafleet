"""``cafleet db`` — database schema management commands."""

import importlib.resources
from pathlib import Path

import click
from alembic import command
from alembic.config import Config
from alembic.runtime.migration import MigrationContext
from alembic.script import ScriptDirectory
from sqlalchemy import create_engine, inspect
from sqlalchemy.engine.url import make_url

from cafleet.config import settings


def _sync_db_url() -> str:
    return str(make_url(settings.database_url).set(drivername="sqlite"))


@click.group()
def db() -> None:
    """Database schema management commands."""


def run_db_init() -> None:
    """Initialize or migrate the registry database to the head revision."""
    sync_url = _sync_db_url()
    db_file_str = make_url(sync_url).database
    if not db_file_str:
        raise click.ClickException("database URL has no file path")
    db_file = Path(db_file_str)

    db_file.parent.mkdir(parents=True, exist_ok=True)

    # ``as_file`` materializes the bundled ``alembic.ini`` to a real path
    # because when cafleet is installed from a zipped wheel ``files(...)``
    # returns a Traversable that Alembic cannot open. Hold the context
    # open across ``command.upgrade`` so the extracted file survives.
    with importlib.resources.as_file(
        importlib.resources.files("cafleet.db") / "alembic.ini"
    ) as ini_path:
        cfg = Config(str(ini_path))
        cfg.set_main_option("sqlalchemy.url", sync_url)

        engine = create_engine(sync_url)
        try:
            with engine.connect() as conn:
                inspector = inspect(conn)
                tables = set(inspector.get_table_names())
                has_alembic_version = "alembic_version" in tables
                non_alembic_tables = tables - {"alembic_version"}

                current_rev: str | None = None
                if has_alembic_version:
                    ctx = MigrationContext.configure(conn)
                    current_rev = ctx.get_current_revision()

            if non_alembic_tables and not has_alembic_version:
                raise click.ClickException(
                    "DB has existing tables but no alembic_version. "
                    "Run `alembic stamp head` manually if you are sure the "
                    "schema matches."
                )

            script = ScriptDirectory.from_config(cfg)
            head_rev = script.get_current_head()

            if current_rev is not None:
                known_revisions = {rev.revision for rev in script.walk_revisions()}
                if current_rev not in known_revisions:
                    raise click.ClickException(
                        f"DB schema is at revision {current_rev} which "
                        f"is unknown to this version of cafleet. "
                        f"Refusing to downgrade automatically."
                    )

            if current_rev == head_rev:
                click.echo(f"Already at head ({head_rev}); nothing to do.")
                return

            old_rev = current_rev or "(empty)"
            command.upgrade(cfg, "head")
            if current_rev is None:
                click.echo(
                    f"Created {db_file} and applied migrations to head ({head_rev})."
                )
            else:
                click.echo(f"Upgraded from {old_rev} to {head_rev}.")
        finally:
            engine.dispose()


@db.command("init")
def init() -> None:
    """Initialize or migrate the registry database to the head revision."""
    run_db_init()
