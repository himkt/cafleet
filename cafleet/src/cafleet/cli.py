"""cafleet CLI."""

import contextlib
import importlib.resources
import json
import os
from pathlib import Path

import click
from alembic import command
from alembic.config import Config
from alembic.runtime.migration import MigrationContext
from alembic.script import ScriptDirectory
from sqlalchemy import create_engine, inspect
from sqlalchemy.engine.url import make_url

from cafleet import broker, output, tmux
from cafleet.coding_agent import CODING_AGENTS, CodingAgentConfig, get_coding_agent
from cafleet.config import settings


def _require_session_id(ctx: click.Context) -> None:
    if not ctx.obj.get("session_id"):
        raise click.ClickException(
            "--session-id <uuid> is required for this subcommand. "
            "Create a session with 'cafleet session create' and pass its id."
        )


@contextlib.contextmanager
def _handle_broker_errors(ctx: click.Context):
    """Convert unexpected broker exceptions to ``Error: ...`` + exit 1.

    ``ClickException`` / ``Exit`` pass through so Click's main loop handles
    them (``ClickException`` formats its own message; ``Exit`` is the signal
    already in flight from e.g. ``_require_session_id``).
    """
    try:
        yield
    except (click.ClickException, click.exceptions.Exit):
        raise
    except Exception as exc:
        click.echo(f"Error: {exc}", err=True)
        ctx.exit(1)


def _sync_db_url() -> str:
    return str(make_url(settings.database_url).set(drivername="sqlite"))


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


@cli.group()
def db() -> None:
    """Database schema management commands."""


@db.command("init")
def init() -> None:
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


@cli.group()
def session() -> None:
    """Session management commands."""


@session.command("create")
@click.option("--label", default=None, help="Optional human-readable label.")
@click.option("--json", "as_json", is_flag=True, help="Output as JSON.")
@click.pass_context
def session_create(ctx: click.Context, label: str | None, as_json: bool) -> None:
    """Create a new session (must be run inside a tmux session)."""
    try:
        tmux.ensure_tmux_available()
        director_ctx = tmux.director_context()
    except tmux.TmuxError as exc:
        raise click.ClickException(
            "cafleet session create must be run inside a tmux session"
        ) from exc

    result = broker.create_session(label=label, director_context=director_ctx)

    if as_json or ctx.obj.get("json_output"):
        click.echo(output.format_json(result))
    else:
        click.echo(output.format_session_create(result))


@session.command("list")
@click.option("--json", "as_json", is_flag=True, help="Output as JSON.")
@click.pass_context
def session_list(ctx: click.Context, as_json: bool) -> None:
    """List all sessions."""
    rows = broker.list_sessions()

    if as_json or ctx.obj.get("json_output"):
        click.echo(output.format_json(rows))
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
        raise click.ClickException(f"session '{session_id}' not found.")

    if as_json or ctx.obj.get("json_output"):
        click.echo(output.format_json(result))
    else:
        click.echo(output.format_session_show(result))


@session.command("delete")
@click.argument("session_id")
def session_delete(session_id: str) -> None:
    """Soft-delete a session and deregister every active agent (idempotent)."""
    result = broker.delete_session(session_id)
    n = result["deregistered_count"]
    click.echo(f"Deleted session {session_id}. Deregistered {n} agents.")


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


@cli.command()
@click.option("--name", required=True, help="Agent name")
@click.option("--description", required=True, help="Agent description")
@click.option("--skills", default=None, help="Skills as JSON string")
@click.pass_context
def register(ctx, name, description, skills):
    """Register a new agent with the broker."""
    _require_session_id(ctx)

    parsed_skills = None
    if skills is not None:
        try:
            parsed_skills = json.loads(skills)
        except json.JSONDecodeError as exc:
            raise click.ClickException(f"Invalid JSON in --skills: {exc}") from exc

    with _handle_broker_errors(ctx):
        result = broker.register_agent(
            ctx.obj["session_id"],
            name,
            description,
            skills=parsed_skills,
        )
        if ctx.obj["json_output"]:
            click.echo(output.format_json(result))
        else:
            click.echo(output.format_register(result))


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--to", required=True, help="Recipient agent ID")
@click.option("--text", required=True, help="Message text")
@click.pass_context
def send(ctx, agent_id, to, text):
    """Send a unicast message to another agent."""
    _require_session_id(ctx)
    with _handle_broker_errors(ctx):
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


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--text", required=True, help="Message text")
@click.pass_context
def broadcast(ctx, agent_id, text):
    """Broadcast a message to all agents."""
    _require_session_id(ctx)
    with _handle_broker_errors(ctx):
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


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--since", default=None, help="Filter tasks since timestamp")
@click.option("--page-size", default=None, type=int, help="Number of tasks")
@click.pass_context
def poll(ctx, agent_id, since, page_size):
    """Poll inbox for messages."""
    _require_session_id(ctx)
    with _handle_broker_errors(ctx):
        result = broker.poll_tasks(
            agent_id,
            since=since,
            page_size=page_size,
        )
        if ctx.obj["json_output"]:
            click.echo(output.format_json(result))
        else:
            click.echo(output.format_task_list(result))


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--task-id", required=True, help="Task ID to acknowledge")
@click.pass_context
def ack(ctx, agent_id, task_id):
    """Acknowledge receipt of a message."""
    _require_session_id(ctx)
    with _handle_broker_errors(ctx):
        result = broker.ack_task(agent_id, task_id)
        if ctx.obj["json_output"]:
            click.echo(output.format_json(result))
        else:
            click.echo("Message acknowledged.")
            click.echo(output.format_task(result))


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--task-id", required=True, help="Task ID to cancel")
@click.pass_context
def cancel(ctx, agent_id, task_id):
    """Cancel (retract) a sent message."""
    _require_session_id(ctx)
    with _handle_broker_errors(ctx):
        result = broker.cancel_task(agent_id, task_id)
        if ctx.obj["json_output"]:
            click.echo(output.format_json(result))
        else:
            click.echo("Task canceled.")
            click.echo(output.format_task(result))


@cli.command("get-task")
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--task-id", required=True, help="Task ID to retrieve")
@click.pass_context
def get_task(ctx, agent_id, task_id):
    """Get details of a specific task."""
    _require_session_id(ctx)
    with _handle_broker_errors(ctx):
        result = broker.get_task(ctx.obj["session_id"], task_id)
        if ctx.obj["json_output"]:
            click.echo(output.format_json(result))
        else:
            click.echo(output.format_task(result))


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--id", "detail_id", default=None, help="Get detail for specific agent")
@click.pass_context
def agents(ctx, agent_id, detail_id):
    """List registered agents or get agent detail."""
    _require_session_id(ctx)
    with _handle_broker_errors(ctx):
        if detail_id:
            result = broker.get_agent(detail_id, ctx.obj["session_id"])
            if result is None:
                raise ValueError(f"Agent {detail_id} not found")
            if ctx.obj["json_output"]:
                click.echo(output.format_json(result))
            else:
                click.echo(output.format_agent(result))
        else:
            agent_list = broker.list_agents(ctx.obj["session_id"])
            if ctx.obj["json_output"]:
                click.echo(output.format_json(agent_list))
            else:
                click.echo(output.format_agent_list(agent_list))


@cli.command()
@click.option("--agent-id", required=True, help="Agent ID")
@click.pass_context
def deregister(ctx, agent_id):
    """Deregister this agent from the broker."""
    _require_session_id(ctx)
    with _handle_broker_errors(ctx):
        deregistered = broker.deregister_agent(agent_id)

    if not deregistered:
        raise click.ClickException(
            f"agent {agent_id} not found or already deregistered."
        )

    if ctx.obj["json_output"]:
        click.echo(output.format_json({"status": "deregistered"}))
    else:
        click.echo("Agent deregistered successfully.")


@cli.group()
def member():
    """Manage tmux-backed member agents (Director only)."""


def _load_authorized_member(
    session_id: str,
    director_agent_id: str,
    member_id: str,
    *,
    placement_missing_msg: str,
) -> tuple[dict, dict]:
    """Load a member's agent + placement, enforcing the cross-Director boundary.

    ``placement_missing_msg`` is the full error body for the "no placement"
    path, because each caller points users at a different follow-up command
    (``cafleet deregister`` from delete; ``cafleet member create`` from
    capture / send-input). Pane-id presence is NOT checked here — delete
    tolerates a pending placement while the others reject it.
    """
    try:
        target = broker.get_agent(member_id, session_id)
    except Exception as exc:
        raise click.ClickException(f"failed to fetch member: {exc}") from exc
    if target is None:
        raise click.ClickException(f"Agent {member_id} not found")
    placement = target["placement"]
    if placement is None:
        raise click.ClickException(placement_missing_msg)
    if placement["director_agent_id"] != director_agent_id:
        raise click.ClickException(
            f"agent {member_id} is not a member of your team "
            f"(director_agent_id={placement['director_agent_id']})."
        )
    return target, placement


def _resolve_prompt(
    ctx: click.Context,
    director_agent_id: str,
    new_agent_id: str,
    prompt_argv: tuple[str, ...],
    coding_agent_config: CodingAgentConfig,
) -> str:
    """Substitute ``session_id`` / ``agent_id`` / ``director_*`` into the spawn prompt.

    Runs ``str.format`` on both the coding-agent default template and any
    user-supplied ``prompt_argv``, so custom prompts must double literal
    braces (``{{`` / ``}}``) to survive the substitution.
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
    """Best-effort rollback of a just-created agent registration."""
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
    type=click.Choice(list(CODING_AGENTS)),
    default="claude",
    show_default=True,
    help="Coding agent to spawn in the tmux pane",
)
@click.argument("prompt_argv", nargs=-1)
@click.pass_context
def member_create(ctx, agent_id, name, description, coding_agent, prompt_argv):
    """Register a new member and spawn its coding agent pane in the Director's window."""
    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]
    coding_agent_config = get_coding_agent(coding_agent)

    try:
        tmux.ensure_tmux_available()
        coding_agent_config.ensure_available()
        director_ctx = tmux.director_context()
    except (tmux.TmuxError, RuntimeError) as exc:
        raise click.ClickException(str(exc)) from exc

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
        raise click.ClickException(f"register failed: {exc}") from exc
    new_agent_id = result["agent_id"]

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

    try:
        placement_view = broker.update_placement_pane_id(new_agent_id, pane_id)
    except Exception as exc:
        # Pane is alive but the registration row is dangling; /exit the pane
        # and roll back the agent so the caller can retry cleanly.
        with contextlib.suppress(tmux.TmuxError):
            tmux.send_exit(target_pane_id=pane_id, ignore_missing=True)
        _rollback_register(
            new_agent_id,
            session_id=session_id,
            reason=f"placement update failed: {exc}",
        )
        ctx.exit(1)
        return

    try:
        tmux.select_layout(target_window_id=director_ctx.window_id)
    except tmux.TmuxError as exc:
        click.echo(f"Warning: select-layout failed: {exc}", err=True)

    result["placement"] = placement_view
    if ctx.obj["json_output"]:
        click.echo(output.format_json(result))
    else:
        click.echo(output.format_member(result))


@member.command("delete")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.option("--member-id", required=True, help="Target member's agent ID")
@click.pass_context
def member_delete(ctx, agent_id, member_id):
    """Deregister a member agent and close its tmux pane."""
    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]

    try:
        tmux.ensure_tmux_available()
    except tmux.TmuxError as exc:
        raise click.ClickException(str(exc)) from exc

    _target, placement = _load_authorized_member(
        session_id,
        agent_id,
        member_id,
        placement_missing_msg=(
            f"agent {member_id} has no placement; use `cafleet deregister` instead"
        ),
    )
    pane_id = placement["tmux_pane_id"]

    # Deregister the registry row first so a send-keys failure leaves a
    # queryable intent ("already gone") rather than a dangling placement.
    try:
        broker.deregister_agent(member_id)
    except Exception as exc:
        raise click.ClickException(f"deregister failed: {exc}") from exc

    if pane_id is None:
        pane_status = "(pending — no pane)"
    else:
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
        try:
            tmux.select_layout(target_window_id=placement["tmux_window_id"])
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
    with _handle_broker_errors(ctx):
        members = broker.list_members(ctx.obj["session_id"], agent_id)
        if ctx.obj["json_output"]:
            click.echo(output.format_json(members))
        else:
            click.echo(output.format_member_list(members))


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
    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]

    try:
        tmux.ensure_tmux_available()
    except tmux.TmuxError as exc:
        raise click.ClickException(str(exc)) from exc

    _target, placement = _load_authorized_member(
        session_id,
        agent_id,
        member_id,
        placement_missing_msg=(
            f"agent {member_id} has no placement row; it was not "
            f"spawned via `cafleet member create`."
        ),
    )
    pane_id = placement["tmux_pane_id"]
    if pane_id is None:
        raise click.ClickException(
            f"member {member_id} has no pane yet (pending placement) "
            f"— nothing to capture."
        )

    try:
        content = tmux.capture_pane(target_pane_id=pane_id, lines=lines)
    except tmux.TmuxError as exc:
        raise click.ClickException(f"capture failed: {exc}") from exc

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "member_agent_id": member_id,
                    "pane_id": pane_id,
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
    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]

    if (choice is None) == (freetext is None):
        raise click.UsageError("Must supply exactly one of --choice or --freetext.")

    if freetext is not None and ("\n" in freetext or "\r" in freetext):
        raise click.UsageError("free text may not contain newlines.")

    try:
        tmux.ensure_tmux_available()
    except tmux.TmuxError as exc:
        raise click.ClickException(str(exc)) from exc

    target, placement = _load_authorized_member(
        session_id,
        agent_id,
        member_id,
        placement_missing_msg=(
            f"agent {member_id} has no placement row; it was not "
            f"spawned via `cafleet member create`."
        ),
    )
    pane_id = placement["tmux_pane_id"]
    if pane_id is None:
        raise click.ClickException(
            f"member {member_id} has no pane yet (pending placement) — nothing to send."
        )

    try:
        if choice is not None:
            tmux.send_choice_key(target_pane_id=pane_id, digit=choice)
            action, value = "choice", str(choice)
        else:
            tmux.send_freetext_and_submit(target_pane_id=pane_id, text=freetext)
            action, value = "freetext", freetext
    except tmux.TmuxError as exc:
        raise click.ClickException(f"send failed: {exc}") from exc

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
