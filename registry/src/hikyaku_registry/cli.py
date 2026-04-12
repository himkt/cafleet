"""hikyaku-registry CLI — schema management and session commands.

Subgroups:
  - ``db``      — Database schema management (init, etc.)
  - ``session`` — Session CRUD (create, list, show, delete)
"""

import importlib.resources
import json
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path

import click
from alembic import command
from alembic.config import Config
from alembic.runtime.migration import MigrationContext
from alembic.script import ScriptDirectory
from sqlalchemy import create_engine, inspect, text
from sqlalchemy.engine.url import make_url
from sqlalchemy.exc import IntegrityError

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


# ---------------------------------------------------------------------------
# session group — sibling of db
# ---------------------------------------------------------------------------


@main.group()
def session() -> None:
    """Session management commands."""


@session.command("create")
@click.option("--label", default=None, help="Optional human-readable label.")
@click.option("--json", "as_json", is_flag=True, help="Output as JSON.")
def session_create(label: str | None, as_json: bool) -> None:
    """Create a new session."""
    sync_url = _sync_db_url()
    session_id = str(uuid.uuid4())
    created_at = datetime.now(timezone.utc).isoformat()

    engine = create_engine(sync_url)
    try:
        with engine.begin() as conn:
            conn.execute(
                text(
                    "INSERT INTO sessions (session_id, label, created_at) "
                    "VALUES (:sid, :label, :created_at)"
                ),
                {"sid": session_id, "label": label, "created_at": created_at},
            )
    finally:
        engine.dispose()

    if as_json:
        click.echo(
            json.dumps(
                {
                    "session_id": session_id,
                    "label": label,
                    "created_at": created_at,
                }
            )
        )
    else:
        click.echo(session_id)


@session.command("list")
@click.option("--json", "as_json", is_flag=True, help="Output as JSON.")
def session_list(as_json: bool) -> None:
    """List all sessions."""
    sync_url = _sync_db_url()
    engine = create_engine(sync_url)
    try:
        with engine.connect() as conn:
            rows = conn.execute(
                text(
                    "SELECT s.session_id, s.label, s.created_at, "
                    "COUNT(CASE WHEN a.status = 'active' THEN 1 END) AS agent_count "
                    "FROM sessions s "
                    "LEFT JOIN agents a ON a.session_id = s.session_id "
                    "GROUP BY s.session_id "
                    "ORDER BY s.created_at"
                )
            ).fetchall()
    finally:
        engine.dispose()

    if as_json:
        data = [
            {
                "session_id": r[0],
                "label": r[1],
                "created_at": r[2],
                "agent_count": r[3],
            }
            for r in rows
        ]
        click.echo(json.dumps(data))
    else:
        if not rows:
            click.echo("No sessions found.")
            return
        click.echo(f"{'SESSION_ID':<40} {'LABEL':<20} {'AGENTS':<8} {'CREATED_AT'}")
        for r in rows:
            sid, lbl, created, count = r
            click.echo(f"{sid:<40} {lbl or '':<20} {count:<8} {created}")


@session.command("show")
@click.argument("session_id")
@click.option("--json", "as_json", is_flag=True, help="Output as JSON.")
def session_show(session_id: str, as_json: bool) -> None:
    """Show details of a single session."""
    sync_url = _sync_db_url()
    engine = create_engine(sync_url)
    try:
        with engine.connect() as conn:
            row = conn.execute(
                text(
                    "SELECT session_id, label, created_at FROM sessions "
                    "WHERE session_id = :sid"
                ),
                {"sid": session_id},
            ).first()
    finally:
        engine.dispose()

    if row is None:
        click.echo(f"Error: session '{session_id}' not found.", err=True)
        sys.exit(1)

    if as_json:
        click.echo(
            json.dumps(
                {
                    "session_id": row[0],
                    "label": row[1],
                    "created_at": row[2],
                }
            )
        )
    else:
        click.echo(f"session_id: {row[0]}")
        click.echo(f"label:      {row[1] or ''}")
        click.echo(f"created_at: {row[2]}")


@session.command("delete")
@click.argument("session_id")
def session_delete(session_id: str) -> None:
    """Delete a session."""
    sync_url = _sync_db_url()
    engine = create_engine(sync_url)
    try:
        with engine.begin() as conn:
            conn.execute(text("PRAGMA foreign_keys = ON"))
            try:
                result = conn.execute(
                    text("DELETE FROM sessions WHERE session_id = :sid"),
                    {"sid": session_id},
                )
            except IntegrityError:
                count = conn.execute(
                    text(
                        "SELECT COUNT(*) FROM agents WHERE session_id = :sid"
                    ),
                    {"sid": session_id},
                ).scalar()
                raise click.UsageError(
                    f"Cannot delete session {session_id}: "
                    f"it still has {count} agent(s) referencing it."
                )
        if result.rowcount == 0:
            click.echo(f"Error: session '{session_id}' not found.", err=True)
            sys.exit(1)
        click.echo(f"Deleted session {session_id}.")
    finally:
        engine.dispose()


if __name__ == "__main__":
    main()
