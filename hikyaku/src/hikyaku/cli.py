"""hikyaku CLI — unified command-line interface.

Subgroups:
  - ``db``      — Database schema management (init, etc.)
  - ``session`` — Session CRUD (create, list, show, delete)
  - ``member``  — Manage tmux-backed member agents (Director only)

Top-level commands:
  env, register, send, broadcast, poll, ack, cancel, get-task, agents, deregister
"""

import asyncio
import importlib.resources
import json
import os
import sys
import uuid
from collections.abc import Coroutine
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import click
from alembic import command
from alembic.config import Config
from alembic.runtime.migration import MigrationContext
from alembic.script import ScriptDirectory
from sqlalchemy import create_engine, inspect, text
from sqlalchemy.engine.url import make_url
from sqlalchemy.exc import IntegrityError

from hikyaku import broker_client as api
from hikyaku import output
from hikyaku.config import settings


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _run(coro: Coroutine) -> Any:
    """Run an async coroutine synchronously."""
    return asyncio.run(coro)


def _require_session_id(ctx: click.Context) -> None:
    """Validate that HIKYAKU_SESSION_ID is set."""
    if not ctx.obj.get("session_id"):
        click.echo(
            "Error: HIKYAKU_SESSION_ID environment variable is required. "
            "Create a session with 'hikyaku session create'.",
            err=True,
        )
        ctx.exit(1)


def _sync_db_url() -> str:
    return str(make_url(settings.database_url).set(drivername="sqlite"))


# ---------------------------------------------------------------------------
# Root group
# ---------------------------------------------------------------------------


@click.group()
@click.option(
    "--json", "json_output", is_flag=True, default=False, help="Output in JSON format"
)
@click.pass_context
def cli(ctx, json_output):
    """Hikyaku — CLI for the A2A message broker."""
    ctx.ensure_object(dict)
    url = os.environ.get("HIKYAKU_URL") or "http://127.0.0.1:8000"
    session_id = os.environ.get("HIKYAKU_SESSION_ID")
    ctx.obj["url"] = url
    ctx.obj["session_id"] = session_id
    ctx.obj["json_output"] = json_output


# ---------------------------------------------------------------------------
# env command
# ---------------------------------------------------------------------------


@cli.command()
@click.pass_context
def env(ctx):
    """Print HIKYAKU_URL and HIKYAKU_SESSION_ID from the environment."""
    click.echo(f"HIKYAKU_URL={ctx.obj['url']}")
    session_id = ctx.obj["session_id"] or ""
    click.echo(f"HIKYAKU_SESSION_ID={session_id}")


# ---------------------------------------------------------------------------
# db subgroup
# ---------------------------------------------------------------------------


@cli.group()
def db() -> None:
    """Database schema management commands."""


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
    # even when ``hikyaku`` is imported from a zipped wheel,
    # where ``files(...)`` would otherwise return a virtual ``Traversable``
    # that Alembic cannot open. The context manager is held open for the
    # entire ``command.upgrade`` call so the extracted file is not
    # cleaned up prematurely.
    with importlib.resources.as_file(
        importlib.resources.files("hikyaku") / "alembic.ini"
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
                        f"is unknown to this version of hikyaku. "
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
# session subgroup
# ---------------------------------------------------------------------------


@cli.group()
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
                    text("SELECT COUNT(*) FROM agents WHERE session_id = :sid"),
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


# ---------------------------------------------------------------------------
# Client commands (require HIKYAKU_URL + HIKYAKU_SESSION_ID)
# ---------------------------------------------------------------------------


@cli.command()
@click.option("--name", required=True, help="Agent name")
@click.option("--description", required=True, help="Agent description")
@click.option("--skills", default=None, help="Skills as JSON string")
@click.pass_context
def register(ctx, name, description, skills):
    """Register a new agent with the broker."""
    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]

    try:
        parsed_skills = None
        if skills is not None:
            try:
                parsed_skills = json.loads(skills)
            except json.JSONDecodeError as e:
                click.echo(f"Error: Invalid JSON in --skills: {e}", err=True)
                ctx.exit(1)
                return

        result = _run(
            api.register_agent(
                ctx.obj["url"],
                name,
                description,
                skills=parsed_skills,
                session_id=session_id,
            )
        )

        if ctx.obj["json_output"]:
            click.echo(output.format_json(result))
        else:
            click.echo(output.format_register(result))
    except Exception as e:
        click.echo(f"Error: {e}", err=True)
        ctx.exit(1)


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--to", required=True, help="Recipient agent ID")
@click.option("--text", required=True, help="Message text")
@click.pass_context
def send(ctx, agent_id, to, text):
    """Send a unicast message to another agent."""
    _require_session_id(ctx)
    try:
        result = _run(
            api.send_message(
                ctx.obj["url"],
                ctx.obj["session_id"],
                agent_id,
                to,
                text,
            )
        )

        if ctx.obj["json_output"]:
            click.echo(output.format_json(result))
        else:
            click.echo("Message sent.")
            click.echo(output.format_task(result))
    except Exception as e:
        click.echo(f"Error: {e}", err=True)
        ctx.exit(1)


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--text", required=True, help="Message text")
@click.pass_context
def broadcast(ctx, agent_id, text):
    """Broadcast a message to all agents."""
    _require_session_id(ctx)
    try:
        result = _run(
            api.broadcast_message(
                ctx.obj["url"],
                ctx.obj["session_id"],
                agent_id,
                text,
            )
        )

        if ctx.obj["json_output"]:
            click.echo(output.format_json(result))
        else:
            click.echo("Broadcast sent.")
            click.echo(output.format_task_list(result))
    except Exception as e:
        click.echo(f"Error: {e}", err=True)
        ctx.exit(1)


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--since", default=None, help="Filter tasks since timestamp")
@click.option("--page-size", default=None, type=int, help="Number of tasks")
@click.pass_context
def poll(ctx, agent_id, since, page_size):
    """Poll inbox for messages."""
    _require_session_id(ctx)
    try:
        result = _run(
            api.poll_tasks(
                ctx.obj["url"],
                ctx.obj["session_id"],
                agent_id,
                since=since,
                page_size=page_size,
            )
        )

        if ctx.obj["json_output"]:
            click.echo(output.format_json(result))
        else:
            click.echo(output.format_task_list(result))
    except Exception as e:
        click.echo(f"Error: {e}", err=True)
        ctx.exit(1)


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--task-id", required=True, help="Task ID to acknowledge")
@click.pass_context
def ack(ctx, agent_id, task_id):
    """Acknowledge receipt of a message."""
    _require_session_id(ctx)
    try:
        result = _run(
            api.ack_task(
                ctx.obj["url"],
                ctx.obj["session_id"],
                agent_id,
                task_id,
            )
        )

        if ctx.obj["json_output"]:
            click.echo(output.format_json(result))
        else:
            click.echo("Message acknowledged.")
            click.echo(output.format_task(result))
    except Exception as e:
        click.echo(f"Error: {e}", err=True)
        ctx.exit(1)


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--task-id", required=True, help="Task ID to cancel")
@click.pass_context
def cancel(ctx, agent_id, task_id):
    """Cancel (retract) a sent message."""
    _require_session_id(ctx)
    try:
        result = _run(
            api.cancel_task(
                ctx.obj["url"],
                ctx.obj["session_id"],
                agent_id,
                task_id,
            )
        )

        if ctx.obj["json_output"]:
            click.echo(output.format_json(result))
        else:
            click.echo("Task canceled.")
            click.echo(output.format_task(result))
    except Exception as e:
        click.echo(f"Error: {e}", err=True)
        ctx.exit(1)


@cli.command("get-task")
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--task-id", required=True, help="Task ID to retrieve")
@click.pass_context
def get_task(ctx, agent_id, task_id):
    """Get details of a specific task."""
    _require_session_id(ctx)
    try:
        result = _run(
            api.get_task(
                ctx.obj["url"],
                ctx.obj["session_id"],
                agent_id,
                task_id,
            )
        )

        if ctx.obj["json_output"]:
            click.echo(output.format_json(result))
        else:
            click.echo(output.format_task(result))
    except Exception as e:
        click.echo(f"Error: {e}", err=True)
        ctx.exit(1)


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--id", "detail_id", default=None, help="Get detail for specific agent")
@click.pass_context
def agents(ctx, agent_id, detail_id):
    """List registered agents or get agent detail."""
    _require_session_id(ctx)
    try:
        result = _run(
            api.list_agents(
                ctx.obj["url"],
                ctx.obj["session_id"],
                caller_id=agent_id,
                agent_id=detail_id,
            )
        )

        if ctx.obj["json_output"]:
            click.echo(output.format_json(result))
        else:
            if isinstance(result, list):
                click.echo(output.format_agent_list(result))
            else:
                click.echo(output.format_agent(result))
    except Exception as e:
        click.echo(f"Error: {e}", err=True)
        ctx.exit(1)


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.pass_context
def deregister(ctx, agent_id):
    """Deregister this agent from the broker."""
    _require_session_id(ctx)
    try:
        _run(
            api.deregister_agent(
                ctx.obj["url"],
                ctx.obj["session_id"],
                agent_id,
            )
        )

        if ctx.obj["json_output"]:
            click.echo(output.format_json({"status": "deregistered"}))
        else:
            click.echo("Agent deregistered successfully.")
    except Exception as e:
        click.echo(f"Error: {e}", err=True)
        ctx.exit(1)


# ---------------------------------------------------------------------------
# member subgroup
# ---------------------------------------------------------------------------


@cli.group()
def member():
    """Manage tmux-backed member agents (Director only)."""


def _resolve_prompt(
    ctx: click.Context, director_agent_id: str, prompt_argv: tuple[str, ...]
) -> str:
    if prompt_argv:
        return " ".join(prompt_argv)
    director = _run(
        api.list_agents(
            ctx.obj["url"],
            ctx.obj["session_id"],
            caller_id=director_agent_id,
            agent_id=director_agent_id,
        )
    )
    return (
        f"Load Skill(hikyaku). Your agent_id is $HIKYAKU_AGENT_ID. "
        f"You are a member of the team led by {director['name']} "
        f"({director_agent_id}). Wait for instructions via "
        f"`hikyaku poll --agent-id $HIKYAKU_AGENT_ID`."
    )


def _rollback_register(broker_url, session_id, director_id, new_agent_id, *, reason):
    """Best-effort rollback: deregister the just-created agent as the Director."""
    click.echo(
        f"Error: {reason}. Rolling back registration of {new_agent_id}.",
        err=True,
    )
    try:
        _run(
            api.deregister_agent(
                broker_url, session_id, new_agent_id, caller_id=director_id
            )
        )
    except Exception as drop_exc:
        click.echo(
            f"WARNING: rollback deregister failed — agent {new_agent_id} is "
            f"orphaned in the registry. Run `hikyaku deregister --agent-id "
            f"{new_agent_id}` manually to clean up. Cause: {drop_exc}",
            err=True,
        )


@member.command("create")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.option("--name", required=True, help="Member name")
@click.option("--description", required=True, help="Member description")
@click.argument("prompt_argv", nargs=-1)
@click.pass_context
def member_create(ctx, agent_id, name, description, prompt_argv):
    """Register a new member and spawn its claude pane in the Director's window."""
    from hikyaku import tmux

    _require_session_id(ctx)
    broker_url = ctx.obj["url"]
    session_id = ctx.obj["session_id"]

    # Pre-flight: must be running inside a tmux session.
    try:
        tmux.ensure_tmux_available()
        director_ctx = tmux.director_context()
    except tmux.TmuxError as exc:
        click.echo(f"Error: {exc}", err=True)
        ctx.exit(1)
        return

    prompt = _resolve_prompt(ctx, agent_id, prompt_argv)

    # Step 1 — register member with pending placement (tmux_pane_id=null).
    try:
        result = _run(
            api.register_agent(
                broker_url,
                name,
                description,
                session_id=session_id,
                director_agent_id=agent_id,
                placement={
                    "director_agent_id": agent_id,
                    "tmux_session": director_ctx.session,
                    "tmux_window_id": director_ctx.window_id,
                    "tmux_pane_id": None,
                },
            )
        )
    except Exception as exc:
        click.echo(f"Error: register failed: {exc}", err=True)
        ctx.exit(1)
        return
    new_agent_id = result["agent_id"]

    # Step 2 — split-window, forwarding env so the spawned claude can reach the broker.
    try:
        pane_id = tmux.split_window(
            target_window_id=director_ctx.window_id,
            env={
                "HIKYAKU_URL": broker_url,
                "HIKYAKU_SESSION_ID": session_id,
                "HIKYAKU_AGENT_ID": new_agent_id,
            },
            claude_prompt=prompt,
        )
    except tmux.TmuxError as exc:
        _rollback_register(
            broker_url,
            session_id,
            agent_id,
            new_agent_id,
            reason=f"tmux split-window failed: {exc}",
        )
        ctx.exit(1)
        return

    # Step 3 — PATCH the pending placement with the real pane_id.
    try:
        placement_view = _run(
            api.patch_placement(
                broker_url,
                session_id,
                director_agent_id=agent_id,
                member_agent_id=new_agent_id,
                pane_id=pane_id,
            )
        )
    except Exception as exc:
        # Placement patch failed: pane is alive but dangling. /exit it, then roll back.
        try:
            tmux.send_exit(target_pane_id=pane_id, ignore_missing=True)
        except tmux.TmuxError:
            pass
        _rollback_register(
            broker_url,
            session_id,
            agent_id,
            new_agent_id,
            reason=f"placement PATCH failed: {exc}",
        )
        ctx.exit(1)
        return

    # Step 4 — rebalance layout (best-effort, non-fatal).
    try:
        tmux.select_layout(target_window_id=director_ctx.window_id)
    except tmux.TmuxError as exc:
        click.echo(f"Warning: select-layout failed: {exc}", err=True)

    result["placement"] = placement_view
    if ctx.obj["json_output"]:
        sanitized = {k: v for k, v in result.items() if k != "session_id"}
        click.echo(output.format_json(sanitized))
    else:
        click.echo(output.format_member(result))


@member.command("delete")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.option("--member-id", required=True, help="Target member's agent ID")
@click.pass_context
def member_delete(ctx, agent_id, member_id):
    """Deregister a member agent and close its tmux pane."""
    from hikyaku import tmux

    _require_session_id(ctx)
    broker_url = ctx.obj["url"]
    session_id = ctx.obj["session_id"]

    try:
        tmux.ensure_tmux_available()
        director_ctx = tmux.director_context()
    except tmux.TmuxError as exc:
        click.echo(f"Error: {exc}", err=True)
        ctx.exit(1)
        return

    # Step 1 — fetch the target agent + placement.
    try:
        target = _run(
            api.list_agents(
                broker_url,
                session_id,
                caller_id=agent_id,
                agent_id=member_id,
            )
        )
    except Exception as exc:
        click.echo(f"Error: failed to fetch member: {exc}", err=True)
        ctx.exit(1)
        return

    placement = target.get("placement")
    if placement is None:
        click.echo(
            f"Error: agent {member_id} has no placement; use `hikyaku deregister` instead",
            err=True,
        )
        ctx.exit(1)
        return

    pane_id = placement.get("tmux_pane_id")

    # Step 2 — deregister the member (BEFORE closing pane).
    try:
        _run(
            api.deregister_agent(broker_url, session_id, member_id, caller_id=agent_id)
        )
    except Exception as exc:
        click.echo(f"Error: deregister failed: {exc}", err=True)
        ctx.exit(1)
        return

    # Step 3 — send /exit to the pane (skip if pending placement).
    pane_status = ""
    if pane_id is not None:
        try:
            tmux.send_exit(target_pane_id=pane_id, ignore_missing=True)
            pane_status = f"{pane_id} (closed)"
        except tmux.TmuxError as exc:
            click.echo(
                f"Warning: send_exit failed for pane {pane_id}: {exc}. "
                f"Kill it manually with `tmux kill-pane -t {pane_id}`.",
                err=True,
            )
            pane_status = f"{pane_id} (send_exit failed)"
    else:
        pane_status = "(pending — no pane)"

    # Step 4 — rebalance layout (skip if pending placement).
    if pane_id is not None:
        try:
            tmux.select_layout(
                target_window_id=placement.get("tmux_window_id", director_ctx.window_id)
            )
        except tmux.TmuxError as exc:
            click.echo(f"Warning: select-layout failed: {exc}", err=True)

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json({"agent_id": member_id, "pane_status": pane_status})
        )
    else:
        click.echo("Member deleted.")
        click.echo(f"  agent_id:  {member_id}")
        click.echo(f"  pane_id:   {pane_status}")


@member.command("list")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.pass_context
def member_list(ctx, agent_id):
    """List member agents managed by this Director."""
    _require_session_id(ctx)
    try:
        members = _run(
            api.list_members(
                ctx.obj["url"],
                ctx.obj["session_id"],
                agent_id,
            )
        )

        if ctx.obj["json_output"]:
            click.echo(output.format_json(members))
        else:
            click.echo(output.format_member_list(members))
    except Exception as e:
        click.echo(f"Error: {e}", err=True)
        ctx.exit(1)


@member.command("capture")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.option("--member-id", required=True, help="Target member's agent ID")
@click.option(
    "--lines",
    type=int,
    default=80,
    show_default=True,
    help="Number of trailing terminal lines to capture",
)
@click.pass_context
def member_capture(ctx, agent_id, member_id, lines):
    """Capture the last N lines of a member pane's terminal buffer."""
    from hikyaku import tmux

    _require_session_id(ctx)
    broker_url = ctx.obj["url"]
    session_id = ctx.obj["session_id"]

    try:
        tmux.ensure_tmux_available()
    except tmux.TmuxError as exc:
        click.echo(f"Error: {exc}", err=True)
        ctx.exit(1)
        return

    # Fetch the target's agent + placement.
    try:
        target = _run(
            api.list_agents(
                broker_url,
                session_id,
                caller_id=agent_id,
                agent_id=member_id,
            )
        )
    except Exception as exc:
        click.echo(f"Error: failed to fetch member: {exc}", err=True)
        ctx.exit(1)
        return

    placement = target.get("placement")
    if placement is None:
        click.echo(
            f"Error: agent {member_id} has no placement row; it was not "
            f"spawned via `hikyaku member create`.",
            err=True,
        )
        ctx.exit(1)
        return
    if placement["director_agent_id"] != agent_id:
        click.echo(
            f"Error: agent {member_id} is not a member of your team "
            f"(director_agent_id={placement['director_agent_id']}).",
            err=True,
        )
        ctx.exit(1)
        return
    if placement.get("tmux_pane_id") is None:
        click.echo(
            f"Error: member {member_id} has no pane yet (pending placement) "
            f"— nothing to capture.",
            err=True,
        )
        ctx.exit(1)
        return

    try:
        content = tmux.capture_pane(
            target_pane_id=placement["tmux_pane_id"], lines=lines
        )
    except tmux.TmuxError as exc:
        click.echo(f"Error: capture failed: {exc}", err=True)
        ctx.exit(1)
        return

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "member_agent_id": member_id,
                    "pane_id": placement["tmux_pane_id"],
                    "lines": lines,
                    "content": content,
                }
            )
        )
    else:
        click.echo(content, nl=False)


if __name__ == "__main__":
    cli()
