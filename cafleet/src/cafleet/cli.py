"""cafleet CLI — unified command-line interface.

Subgroups:
  - ``db``      — Database schema management (init, etc.)
  - ``session`` — Session CRUD (create, list, show, delete)
  - ``member``  — Manage tmux-backed member agents (Director only)

Top-level commands:
  register, send, broadcast, poll, ack, cancel, get-task, agents, deregister
"""

import importlib.resources
import json
import os
import sys
from pathlib import Path

import click
from alembic import command
from alembic.config import Config
from alembic.runtime.migration import MigrationContext
from alembic.script import ScriptDirectory
from sqlalchemy import create_engine, inspect
from sqlalchemy.engine.url import make_url

from cafleet import broker, output, tmux
from cafleet.coding_agent import CodingAgentConfig, get_coding_agent
from cafleet.config import settings
from cafleet.tmux import TmuxError


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _require_session_id(ctx: click.Context) -> None:
    """Validate that --session-id was provided on the root group."""
    if not ctx.obj.get("session_id"):
        click.echo(
            "Error: --session-id <uuid> is required for this subcommand. "
            "Create a session with 'cafleet session create' and pass its id.",
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
@click.option(
    "--session-id",
    "session_id",
    default=None,
    help="Session ID (UUID); required for client subcommands.",
)
@click.pass_context
def cli(ctx, json_output, session_id):
    """CAFleet — CLI for the A2A-inspired message broker."""
    ctx.ensure_object(dict)
    ctx.obj["session_id"] = session_id
    ctx.obj["json_output"] = json_output


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
    # even when ``cafleet`` is imported from a zipped wheel,
    # where ``files(...)`` would otherwise return a virtual ``Traversable``
    # that Alembic cannot open. The context manager is held open for the
    # entire ``command.upgrade`` call so the extracted file is not
    # cleaned up prematurely.
    with importlib.resources.as_file(
        importlib.resources.files("cafleet") / "alembic.ini"
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
                        f"is unknown to this version of cafleet. "
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
@click.pass_context
def session_create(ctx: click.Context, label: str | None, as_json: bool) -> None:
    """Create a new session (must be run inside a tmux session).

    Atomically bootstraps the session, registers the root Director with a
    placement pointing at the current tmux pane, and seeds the built-in
    Administrator — all in one transaction (design 0000026).
    """
    try:
        tmux.ensure_tmux_available()
        director_ctx = tmux.director_context()
    except TmuxError:
        click.echo(
            "Error: cafleet session create must be run inside a tmux session",
            err=True,
        )
        ctx.exit(1)
        return

    result = broker.create_session(label=label, director_context=director_ctx)

    if as_json or ctx.obj.get("json_output"):
        click.echo(json.dumps(result))
    else:
        click.echo(output.format_session_create(result))


@session.command("list")
@click.option("--json", "as_json", is_flag=True, help="Output as JSON.")
@click.pass_context
def session_list(ctx: click.Context, as_json: bool) -> None:
    """List all sessions."""
    rows = broker.list_sessions()

    if as_json or ctx.obj.get("json_output"):
        click.echo(json.dumps(rows))
    else:
        if not rows:
            click.echo("No sessions found.")
            return
        click.echo(f"{'SESSION_ID':<40} {'LABEL':<20} {'AGENTS':<8} {'CREATED_AT'}")
        for r in rows:
            click.echo(
                f"{r['session_id']:<40} {r['label'] or '':<20} "
                f"{r['agent_count']:<8} {r['created_at']}"
            )


@session.command("show")
@click.argument("session_id")
@click.option("--json", "as_json", is_flag=True, help="Output as JSON.")
@click.pass_context
def session_show(ctx: click.Context, session_id: str, as_json: bool) -> None:
    """Show details of a single session."""
    result = broker.get_session(session_id)

    if result is None:
        click.echo(f"Error: session '{session_id}' not found.", err=True)
        sys.exit(1)

    if as_json or ctx.obj.get("json_output"):
        click.echo(json.dumps(result))
    else:
        click.echo(f"session_id: {result['session_id']}")
        click.echo(f"label:      {result['label'] or ''}")
        click.echo(f"created_at: {result['created_at']}")


@session.command("delete")
@click.argument("session_id")
def session_delete(session_id: str) -> None:
    """Soft-delete a session and deregister every active agent (idempotent)."""
    result = broker.delete_session(session_id)
    n = result["deregistered_count"]
    click.echo(f"Deleted session {session_id}. Deregistered {n} agents.")


# ---------------------------------------------------------------------------
# server (does NOT require --session-id; supplying one is silently accepted)
# ---------------------------------------------------------------------------


@cli.command("server")
@click.option(
    "--host",
    default=settings.broker_host,
    show_default=True,
    help="Bind address (override via flag or CAFLEET_BROKER_HOST env var).",
)
@click.option(
    "--port",
    default=settings.broker_port,
    show_default=True,
    type=int,
    help="Bind port (override via flag or CAFLEET_BROKER_PORT env var).",
)
def server(host: str, port: int) -> None:
    """Start the admin WebUI FastAPI server."""
    import uvicorn

    uvicorn.run(
        "cafleet.server:app",
        host=host,
        port=port,
    )


# ---------------------------------------------------------------------------
# Client commands (require --session-id)
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

        result = broker.register_agent(
            session_id,
            name,
            description,
            skills=parsed_skills,
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
        result = broker.send_message(
            ctx.obj["session_id"],
            agent_id,
            to,
            text,
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
        result = broker.broadcast_message(
            ctx.obj["session_id"],
            agent_id,
            text,
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
        result = broker.poll_tasks(
            agent_id,
            since=since,
            page_size=page_size,
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
        result = broker.ack_task(agent_id, task_id)

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
        result = broker.cancel_task(agent_id, task_id)

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
        result = broker.get_task(ctx.obj["session_id"], task_id)

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
        if detail_id:
            result = broker.get_agent(detail_id, ctx.obj["session_id"])
            if result is None:
                raise ValueError(f"Agent {detail_id} not found")
            if ctx.obj["json_output"]:
                click.echo(output.format_json(result))
            else:
                click.echo(output.format_agent(result))
        else:
            result = broker.list_agents(ctx.obj["session_id"])
            if ctx.obj["json_output"]:
                click.echo(output.format_json(result))
            else:
                click.echo(output.format_agent_list(result))
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
        deregistered = broker.deregister_agent(agent_id)
    except broker.AdministratorProtectedError as exc:
        click.echo(f"Error: {exc}", err=True)
        ctx.exit(1)
    except Exception as e:
        click.echo(f"Error: {e}", err=True)
        ctx.exit(1)

    if not deregistered:
        click.echo(
            f"Error: agent {agent_id} not found or already deregistered.",
            err=True,
        )
        ctx.exit(1)

    if ctx.obj["json_output"]:
        click.echo(output.format_json({"status": "deregistered"}))
    else:
        click.echo("Agent deregistered successfully.")


# ---------------------------------------------------------------------------
# member subgroup
# ---------------------------------------------------------------------------


@cli.group()
def member():
    """Manage tmux-backed member agents (Director only)."""


def _resolve_prompt(
    ctx: click.Context,
    director_agent_id: str,
    new_agent_id: str,
    prompt_argv: tuple[str, ...],
    coding_agent_config: CodingAgentConfig,
) -> str:
    """Resolve the spawn prompt for a new member agent.

    Joins a user-supplied ``prompt_argv`` (or falls back to the coding
    agent's default template) and runs ``str.format`` on the result with
    ``session_id`` / ``agent_id`` / ``director_name`` / ``director_agent_id``
    as kwargs. Applies to BOTH the default template and custom prompts, so
    callers may embed those placeholders in custom prompts. Literal ``{``
    or ``}`` characters in a custom prompt must be doubled (``{{`` / ``}}``)
    to survive ``.format``.
    """
    session_id = ctx.obj["session_id"]
    director = broker.get_agent(director_agent_id, session_id)
    if director is None:
        raise click.UsageError(f"Director agent {director_agent_id} not found")
    template = (
        " ".join(prompt_argv)
        if prompt_argv
        else coding_agent_config.default_prompt_template
    )
    try:
        return template.format(
            session_id=session_id,
            agent_id=new_agent_id,
            director_name=director["name"],
            director_agent_id=director_agent_id,
        )
    except KeyError as exc:
        raise click.UsageError(
            f"Unknown placeholder {exc} in custom prompt. "
            "Supported placeholders: {session_id}, {agent_id}, "
            "{director_name}, {director_agent_id}. "
            "Double literal braces ({{, }}) to keep them as text."
        ) from exc
    except (ValueError, IndexError, AttributeError) as exc:
        raise click.UsageError(
            f"Malformed custom prompt: {exc}. "
            "Double literal braces ({{, }}) to keep them as text."
        ) from exc


def _rollback_register(new_agent_id, *, session_id, reason):
    """Best-effort rollback: deregister the just-created agent."""
    click.echo(
        f"Error: {reason}. Rolling back registration of {new_agent_id}.",
        err=True,
    )
    try:
        broker.deregister_agent(new_agent_id)
    except Exception as drop_exc:
        click.echo(
            f"WARNING: rollback deregister failed — agent {new_agent_id} is "
            f"orphaned in the registry. Run `cafleet --session-id {session_id} "
            f"deregister --agent-id {new_agent_id}` manually to clean up. "
            f"Cause: {drop_exc}",
            err=True,
        )


@member.command("create")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.option("--name", required=True, help="Member name")
@click.option("--description", required=True, help="Member description")
@click.option(
    "--coding-agent",
    type=click.Choice(["claude", "codex"]),
    default="claude",
    show_default=True,
    help="Coding agent to spawn in the tmux pane",
)
@click.argument("prompt_argv", nargs=-1)
@click.pass_context
def member_create(ctx, agent_id, name, description, coding_agent, prompt_argv):
    """Register a new member and spawn its coding agent pane in the Director's window."""
    from cafleet import tmux

    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]
    coding_agent_config = get_coding_agent(coding_agent)

    # Pre-flight: must be running inside a tmux session.
    try:
        tmux.ensure_tmux_available()
        coding_agent_config.ensure_available()
        director_ctx = tmux.director_context()
    except (tmux.TmuxError, RuntimeError) as exc:
        click.echo(f"Error: {exc}", err=True)
        ctx.exit(1)
        return

    # Step 1 — register member with pending placement (tmux_pane_id=null).
    try:
        result = broker.register_agent(
            session_id,
            name,
            description,
            placement={
                "director_agent_id": agent_id,
                "tmux_session": director_ctx.session,
                "tmux_window_id": director_ctx.window_id,
                "tmux_pane_id": None,
                "coding_agent": coding_agent_config.name,
            },
        )
    except Exception as exc:
        click.echo(f"Error: register failed: {exc}", err=True)
        ctx.exit(1)
        return
    new_agent_id = result["agent_id"]

    # Resolve the prompt now that we know the new member's agent_id so the
    # template can bake the literal UUIDs for session_id and agent_id.
    try:
        prompt = _resolve_prompt(
            ctx, agent_id, new_agent_id, prompt_argv, coding_agent_config
        )
    except click.UsageError as exc:
        _rollback_register(
            new_agent_id,
            session_id=session_id,
            reason=f"prompt resolution failed: {exc}",
        )
        ctx.exit(1)
        return

    # Step 2 — split-window, forwarding only CAFLEET_DATABASE_URL when set.
    try:
        fwd_env: dict[str, str] = {}
        db_url = os.environ.get("CAFLEET_DATABASE_URL")
        if db_url:
            fwd_env["CAFLEET_DATABASE_URL"] = db_url
        pane_id = tmux.split_window(
            target_window_id=director_ctx.window_id,
            env=fwd_env,
            command=coding_agent_config.build_command(prompt, display_name=name),
        )
    except tmux.TmuxError as exc:
        _rollback_register(
            new_agent_id,
            session_id=session_id,
            reason=f"tmux split-window failed: {exc}",
        )
        ctx.exit(1)
        return

    # Step 3 — update the pending placement with the real pane_id.
    try:
        placement_view = broker.update_placement_pane_id(new_agent_id, pane_id)
    except Exception as exc:
        # Placement update failed: pane is alive but dangling. /exit it, then roll back.
        try:
            tmux.send_exit(target_pane_id=pane_id, ignore_missing=True)
        except tmux.TmuxError:
            pass
        _rollback_register(
            new_agent_id,
            session_id=session_id,
            reason=f"placement update failed: {exc}",
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
    from cafleet import tmux

    _require_session_id(ctx)
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
        target = broker.get_agent(member_id, session_id)
        if target is None:
            raise ValueError(f"Agent {member_id} not found")
    except Exception as exc:
        click.echo(f"Error: failed to fetch member: {exc}", err=True)
        ctx.exit(1)
        return

    placement = target.get("placement")
    if placement is None:
        click.echo(
            f"Error: agent {member_id} has no placement; use `cafleet deregister` instead",
            err=True,
        )
        ctx.exit(1)
        return

    pane_id = placement.get("tmux_pane_id")

    # Step 2 — deregister the member (BEFORE closing pane).
    try:
        broker.deregister_agent(member_id)
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
        members = broker.list_members(ctx.obj["session_id"], agent_id)

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
    from cafleet import tmux

    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]

    try:
        tmux.ensure_tmux_available()
    except tmux.TmuxError as exc:
        click.echo(f"Error: {exc}", err=True)
        ctx.exit(1)
        return

    # Fetch the target's agent + placement.
    try:
        target = broker.get_agent(member_id, session_id)
        if target is None:
            raise ValueError(f"Agent {member_id} not found")
    except Exception as exc:
        click.echo(f"Error: failed to fetch member: {exc}", err=True)
        ctx.exit(1)
        return

    placement = target.get("placement")
    if placement is None:
        click.echo(
            f"Error: agent {member_id} has no placement row; it was not "
            f"spawned via `cafleet member create`.",
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


@member.command("send-input")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.option("--member-id", required=True, help="Target member's agent ID")
@click.option(
    "--choice",
    type=click.IntRange(1, 3),
    default=None,
    help="Select option 1, 2, or 3. Mutually exclusive with --freetext.",
)
@click.option(
    "--freetext",
    type=str,
    default=None,
    help='Send "4" + literal text + Enter. Mutually exclusive with --choice.',
)
@click.pass_context
def member_send_input(ctx, agent_id, member_id, choice, freetext):
    """Safely forward a restricted keystroke to a member pane."""
    from cafleet import tmux

    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]

    if (choice is None) == (freetext is None):
        click.echo(
            "Error: Must supply exactly one of --choice or --freetext.",
            err=True,
        )
        ctx.exit(2)
        return

    if freetext is not None and ("\n" in freetext or "\r" in freetext):
        click.echo("Error: free text may not contain newlines.", err=True)
        ctx.exit(2)
        return

    try:
        tmux.ensure_tmux_available()
    except tmux.TmuxError as exc:
        click.echo(f"Error: {exc}", err=True)
        ctx.exit(1)
        return

    target = broker.get_agent(member_id, session_id)
    if target is None:
        click.echo(f"Error: Agent {member_id} not found", err=True)
        ctx.exit(1)
        return

    placement = target.get("placement")
    if placement is None:
        click.echo(
            f"Error: agent {member_id} has no placement row; it was not "
            f"spawned via `cafleet member create`.",
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
    pane_id = placement.get("tmux_pane_id")
    if pane_id is None:
        click.echo(
            f"Error: member {member_id} has no pane yet (pending placement) "
            f"— nothing to send.",
            err=True,
        )
        ctx.exit(1)
        return

    try:
        if choice is not None:
            tmux.send_choice_key(target_pane_id=pane_id, digit=choice)
            action, value = "choice", str(choice)
        else:
            tmux.send_freetext_and_submit(target_pane_id=pane_id, text=freetext)
            action, value = "freetext", freetext
    except tmux.TmuxError as exc:
        click.echo(f"Error: send failed: {exc}", err=True)
        ctx.exit(1)
        return

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "member_agent_id": member_id,
                    "pane_id": pane_id,
                    "action": action,
                    "value": value,
                }
            )
        )
    else:
        label = f"choice {value}" if action == "choice" else "free text"
        click.echo(f"Sent {label} to member {target['name']} ({pane_id}).")


if __name__ == "__main__":
    cli()
