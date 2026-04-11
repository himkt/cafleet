"""hikyaku-registry CLI — schema management commands.

Currently only ``db init`` is implemented (v1). The ``db`` subgroup is
scaffolded so future commands (``db current``, ``db revision``,
``db downgrade``) can be added later without restructuring.
"""

import importlib.resources
import sys
from pathlib import Path

import click
from alembic import command
from alembic.config import Config
from alembic.runtime.migration import MigrationContext
from alembic.script import ScriptDirectory
from sqlalchemy import create_engine, inspect
from sqlalchemy.engine.url import make_url

from hikyaku_registry.config import settings


@click.group()
def main() -> None:
    """hikyaku-registry administrative CLI."""


@main.group()
def db() -> None:
    """Database schema management commands."""


def _sync_db_url() -> str:
    return str(make_url(settings.database_url).set(drivername="sqlite"))


@db.command("init")
def init() -> None:
    """Initialize or migrate the registry database schema to head.

    Idempotent across six states:
      1. DB file missing      -> create parent dirs, upgrade
      2. Empty schema         -> upgrade
      3. At head              -> no-op
      4. Behind head          -> upgrade to head
      5. Ahead of head        -> error (refuse to downgrade)
      6. Legacy (no version)  -> error (require manual stamp)
    """
    sync_url = _sync_db_url()
    db_file_str = make_url(sync_url).database
    if not db_file_str:
        click.echo("ERROR: database URL has no file path", err=True)
        sys.exit(1)
    db_file = Path(db_file_str)

    db_file.parent.mkdir(parents=True, exist_ok=True)

    # ``importlib.resources.as_file`` guarantees a real filesystem path
    # even when ``hikyaku_registry`` is imported from a zipped wheel,
    # where ``files(...)`` would otherwise return a virtual ``Traversable``
    # that Alembic cannot open. The context manager is held open for the
    # entire ``command.upgrade`` call so the extracted file is not
    # cleaned up prematurely.
    with importlib.resources.as_file(
        importlib.resources.files("hikyaku_registry") / "alembic.ini"
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
                click.echo(
                    "ERROR: DB has existing tables but no alembic_version. "
                    "Run `alembic stamp head` manually if you are sure the "
                    "schema matches.",
                    err=True,
                )
                sys.exit(1)

            script = ScriptDirectory.from_config(cfg)
            head_rev = script.get_current_head()

            if current_rev is not None:
                known_revisions = {rev.revision for rev in script.walk_revisions()}
                if current_rev not in known_revisions:
                    click.echo(
                        f"ERROR: DB schema is at revision {current_rev} which "
                        f"is unknown to this version of hikyaku-registry. "
                        f"Refusing to downgrade automatically.",
                        err=True,
                    )
                    sys.exit(1)

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


if __name__ == "__main__":
    main()
