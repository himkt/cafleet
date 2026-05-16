"""cafleet CLI."""

import contextlib
import functools
import importlib.resources
import json
import os
import shutil
from collections.abc import Callable
from pathlib import Path
from typing import NoReturn

import click
from alembic import command
from alembic.config import Config
from alembic.runtime.migration import MigrationContext
from alembic.script import ScriptDirectory
from sqlalchemy import create_engine, inspect
from sqlalchemy.engine.url import make_url

from cafleet import base_dir, broker, output, tmux
from cafleet.config import settings

_CLAUDE_BINARY = "claude"
_CODEX_BINARY = "codex"
_MEMBER_PROMPT_TEMPLATE = (
    "Member of cafleet session {session_id} "
    "(agent={agent_id}, director={director_agent_id}).\n"
    "Load skill 'cafleet'. Bash auto-approves. Poll: "
    "cafleet --session-id {session_id} message poll --agent-id {agent_id}"
)


def _build_claude_command(prompt: str, *, display_name: str) -> list[str]:
    return [
        _CLAUDE_BINARY,
        "--permission-mode",
        "dontAsk",
        "--name",
        display_name,
        prompt,
    ]


def _build_codex_command(prompt: str) -> list[str]:
    return [
        _CODEX_BINARY,
        "--ask-for-approval",
        "never",
        "--sandbox",
        "workspace-write",
        prompt,
    ]


def _ensure_coding_agent_available(binary_name: str) -> None:
    if shutil.which(binary_name) is None:
        raise RuntimeError(f"binary {binary_name} not found on PATH")


def _ensure_tmux_or_die() -> None:
    try:
        tmux.ensure_tmux_available()
    except tmux.TmuxError as exc:
        raise click.ClickException(str(exc)) from exc


_full_flag = click.option("--full", "full", is_flag=True, default=False, hidden=True)
_quiet_flag = click.option("--quiet", "quiet", is_flag=True, default=False, hidden=True)


def _full_flag_with_help(help_text: str):
    return click.option(
        "--full", "full", is_flag=True, default=False, hidden=True, help=help_text
    )


def _quiet_flag_with_help(help_text: str):
    return click.option(
        "--quiet", "quiet", is_flag=True, default=False, hidden=True, help=help_text
    )


def _director_member_options(func):
    func = click.option("--member-id", required=True, help="Target member's agent ID")(
        func
    )
    return click.option("--agent-id", required=True, help="Director's agent ID")(func)


def _require_session_id(ctx: click.Context) -> None:
    if not ctx.obj["session_id"]:
        raise click.ClickException(
            "--session-id <uuid> is required for this subcommand. "
            "Create a session with 'cafleet session create' and pass its id."
        )


def _client_command(
    *,
    requires_agent_session: bool = False,
    text_formatter: Callable[..., str] | None = None,
    truncates_task_text: bool = False,
    renders_agent_card: bool = False,
):
    """Subsume the boilerplate blocks shared by client subcommands.

    Branches:
    - ``truncates_task_text=True`` → JSON output goes through
      ``render_tasks_in_result`` + ``truncate_task_text``; text formatter is
      called as ``text_formatter(result, full=, quiet=)``.
    - ``renders_agent_card=True`` (Surface 18) → JSON output goes through
      ``render_agents_in_result``; text formatter is called as
      ``text_formatter(result, full=)``.
    - Neither → JSON output is the raw broker result; text formatter is
      called as ``text_formatter(result)``.

    The two ``renders_*`` flags are mutually exclusive — a command renders
    tasks OR agent cards, never both.
    """

    def decorator(func):
        @functools.wraps(func)
        def wrapper(ctx, *args, **kwargs):
            _require_session_id(ctx)
            session_id = ctx.obj["session_id"]
            try:
                if requires_agent_session:
                    agent_id = kwargs.get("agent_id")
                    if agent_id is None:
                        raise click.ClickException(
                            "_client_command(requires_agent_session=True) but no "
                            "'agent_id' kwarg was passed. Check the @click.option "
                            "declaration on this command."
                        )
                    if not broker.verify_agent_session(agent_id, session_id):
                        raise click.ClickException(
                            f"agent {agent_id} is not a member of session {session_id}."
                        )
                result = func(ctx, *args, **kwargs)
                full = kwargs.get("full", False)
                pretty = ctx.obj["pretty"]
                if truncates_task_text:
                    output.truncate_task_text(result, full=full)
                    rendered = output.render_tasks_in_result(result, full=full)
                elif renders_agent_card:
                    rendered = output.render_agents_in_result(result, full=full)
                else:
                    rendered = result
                if ctx.obj["json_output"]:
                    click.echo(output.format_json(rendered, pretty=pretty))
                elif text_formatter is not None:
                    if truncates_task_text:
                        extra = {"quiet": kwargs["quiet"]} if "quiet" in kwargs else {}
                        click.echo(text_formatter(result, full=full, **extra))
                    elif renders_agent_card:
                        click.echo(text_formatter(result, full=full))
                    else:
                        click.echo(text_formatter(result))
                else:
                    click.echo(output.format_json(rendered, pretty=pretty))
            except click.ClickException:
                raise
            except Exception as exc:
                raise click.ClickException(str(exc)) from exc
            return result

        return wrapper

    return decorator


def _sync_db_url() -> str:
    return str(make_url(settings.database_url).set(drivername="sqlite"))


@click.group()
@click.version_option(package_name="cafleet", message="cafleet %(version)s")
@click.option(
    "--json", "json_output", is_flag=True, default=False, help="Output in JSON format"
)
@click.option(
    "--pretty",
    "pretty",
    is_flag=True,
    default=False,
    help="Switch JSON output from compact (default) to indent=2.",
)
@click.option(
    "--session-id",
    "session_id",
    default=None,
    help="Session ID (UUID); required for client subcommands.",
)
@click.pass_context
def cli(ctx, json_output, pretty, session_id):
    """CAFleet — CLI for the message broker and agent registry."""
    ctx.ensure_object(dict)
    ctx.obj["session_id"] = session_id
    ctx.obj["json_output"] = json_output
    ctx.obj["pretty"] = pretty


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
@click.option(
    "--coding-agent",
    "coding_agent",
    type=click.Choice(["claude", "codex"]),
    default="claude",
    show_default=True,
    help="Coding agent (claude or codex).",
)
@click.option("--json", "as_json", is_flag=True, help="Output as JSON.")
@_full_flag
@click.pass_context
def session_create(
    ctx: click.Context,
    label: str | None,
    coding_agent: str,
    as_json: bool,
    full: bool,
) -> None:
    """Create a new session (must be run inside a tmux session)."""
    try:
        tmux.ensure_tmux_available()
        director_ctx = tmux.director_context()
    except tmux.TmuxError as exc:
        raise click.ClickException(
            "cafleet session create must be run inside a tmux session"
        ) from exc

    result = broker.create_session(
        label=label,
        director_context=director_ctx,
        coding_agent=coding_agent,
    )

    if as_json or ctx.obj["json_output"]:
        click.echo(output.format_json(result, pretty=ctx.obj["pretty"]))
    else:
        click.echo(output.format_session_create(result, full=full))


@session.command("list")
@click.option("--json", "as_json", is_flag=True, help="Output as JSON.")
@click.pass_context
def session_list(ctx: click.Context, as_json: bool) -> None:
    """List all sessions."""
    rows = broker.list_sessions()

    if as_json or ctx.obj["json_output"]:
        click.echo(output.format_json(rows, pretty=ctx.obj["pretty"]))
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

    if as_json or ctx.obj["json_output"]:
        click.echo(output.format_json(result, pretty=ctx.obj["pretty"]))
    else:
        lines = [
            f"session_id: {result['session_id']}",
            f"label:      {result['label'] or ''}",
            f"created_at: {result['created_at']}",
        ]
        if result["deleted_at"] is not None:
            lines.append(f"deleted_at: {result['deleted_at']}")
        click.echo("\n".join(lines))


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
    help="Bind address.",
)
@click.option(
    "--port",
    default=settings.broker_port,
    show_default=True,
    type=int,
    help="Bind port.",
)
def server(host: str, port: int) -> None:
    """Start the admin WebUI FastAPI server."""
    import uvicorn

    uvicorn.run(
        "cafleet.server:app",
        host=host,
        port=port,
    )


@cli.command("doctor")
@click.pass_context
def doctor(ctx) -> None:
    """Print the calling pane's tmux session/window/pane identifiers."""
    _ensure_tmux_or_die()

    try:
        director_ctx = tmux.director_context()
    except tmux.TmuxError as exc:
        raise click.ClickException(str(exc)) from exc

    tmux_pane_env = os.environ["TMUX_PANE"]

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "tmux": {
                        "session_name": director_ctx.session,
                        "window_id": director_ctx.window_id,
                        "pane_id": director_ctx.pane_id,
                        "tmux_pane_env": tmux_pane_env,
                    }
                },
                pretty=ctx.obj["pretty"],
            )
        )
    else:
        click.echo("tmux:")
        click.echo(f"  session_name:  {director_ctx.session}")
        click.echo(f"  window_id:     {director_ctx.window_id}")
        click.echo(f"  pane_id:       {director_ctx.pane_id}")
        click.echo(f"  TMUX_PANE:     {tmux_pane_env}")


@cli.group("base-dir")
def base_dir_group() -> None:
    """Resolve or persist the BASE output-root anchor."""


@base_dir_group.command("resolve")
@click.argument("task_name", required=False)
@click.option(
    "--json",
    "as_json",
    is_flag=True,
    default=False,
    help="Emit machine-parseable JSON instead of human-readable text.",
)
@click.pass_context
def base_dir_resolve(ctx: click.Context, task_name: str | None, as_json: bool) -> None:
    """Resolve `${BASE}` non-interactively.

    With no positional argument: the no-positional branch infers BASE from
    CWD (auto-writing the anchor on first call) or signals
    ``needs-user-input`` when CWD is ``$HOME`` / under ``$HOME/.claude`` and
    no usable anchor exists.

    With a positional ``TASK_NAME`` (relative path like ``researches/<slug>``
    or ``design-docs/<NNNNNNN>-<slug>``, or an absolute path inside such a
    folder): engages the task-scope branch — auto-creates the task folder,
    writes a per-task anchor with ``source: "task-scope"``, and returns the
    task folder as ``${BASE}``. When CWD has no ``.git`` ancestor, the task-
    scope branch exits 1 with a plain-text stderr message and emits no JSON
    payload, even with ``--json``.
    """
    try:
        result = base_dir.resolve(task_name=task_name)
    except RuntimeError as exc:
        # Plain-text stderr exit per design 0000060 §Spec 2 *No-repo-root failure mode* —
        # no JSON payload, even when --json is passed.
        click.echo(str(exc), err=True)
        ctx.exit(1)
    except (base_dir.AnchorError, OSError) as exc:
        raise click.ClickException(str(exc)) from exc

    if as_json or ctx.obj["json_output"]:
        click.echo(output.format_json(result, pretty=ctx.obj["pretty"]))
        return

    click.echo(f"status: {result['status']}")
    if result.get("base"):
        click.echo(f"base:   {result['base']}")
    if result.get("source"):
        click.echo(f"source: {result['source']}")
    if result.get("anchor"):
        click.echo(f"anchor: {result['anchor']}")
    if result.get("candidates"):
        click.echo("candidates:")
        for c in result["candidates"]:
            click.echo(f"  - {c}")
    if result.get("task_name"):
        click.echo(f"task_name: {result['task_name']}")


@base_dir_group.command("record")
@click.option(
    "--base",
    "base_arg",
    required=True,
    help="Absolute path to record as the BASE output-root.",
)
@click.option(
    "--source",
    "source_arg",
    type=click.Choice(["askuserquestion", "cwd-inference"]),
    required=True,
    help="How the BASE was determined.",
)
@click.pass_context
def base_dir_record(ctx: click.Context, base_arg: str, source_arg: str) -> None:
    """Persist a `${BASE}/.cafleet-base-dir.json` anchor (idempotent on match)."""
    base_path = Path(base_arg)
    anchor_existed = (base_path / base_dir.ANCHOR_FILENAME).is_file()

    try:
        anchor = base_dir.record(base_arg, source=source_arg)
    except (ValueError, base_dir.AnchorError, OSError) as exc:
        raise click.ClickException(str(exc)) from exc

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json({"anchor": str(anchor)}, pretty=ctx.obj["pretty"])
        )
    else:
        click.echo(f"anchor: {anchor}")

    if not anchor_existed and base_dir.is_git_repo_root(base_path):
        click.echo(
            "tip: this BASE is a git-repo root. Add "
            f"'{base_dir.ANCHOR_FILENAME}' to your global git excludes "
            "(~/.config/git/ignore) so the anchor never shows up in git status.",
            err=True,
        )


@cli.group()
def agent() -> None:
    """Agent registry commands."""


@cli.group()
def message() -> None:
    """Message broker commands."""


@agent.command("register")
@click.option("--name", required=True, help="Agent name")
@click.option("--description", required=True, help="Agent description")
@click.option("--skills", default=None, help="Skills as JSON string")
@click.pass_context
@_client_command(text_formatter=output.format_register)
def agent_register(ctx, name, description, skills):
    """Register a new agent with the broker."""
    parsed_skills = None
    if skills is not None:
        try:
            parsed_skills = json.loads(skills)
        except json.JSONDecodeError as exc:
            raise click.ClickException(f"Invalid JSON in --skills: {exc}") from exc
    return broker.register_agent(
        ctx.obj["session_id"],
        name,
        description,
        skills=parsed_skills,
    )


@message.command("send")
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--to", required=True, help="Recipient agent ID")
@click.option("--text", required=True, help="Message text")
@_full_flag
@_quiet_flag
@click.pass_context
@_client_command(
    text_formatter=lambda r, *, full, quiet: (
        r["task"]["task_id"][:8]
        if quiet
        else "Message sent.\n" + output.format_task(r, full=full)
    ),
    truncates_task_text=True,
)
def message_send(ctx, agent_id, to, text, full, quiet):
    """Send a unicast message to another agent."""
    return broker.send_message(
        ctx.obj["session_id"],
        agent_id,
        to,
        text,
    )


@message.command("broadcast")
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--text", required=True, help="Message text")
@_full_flag
@click.pass_context
@_client_command(
    text_formatter=lambda r, *, full: (
        output.format_task(r[0]["task"], full=True)
        if full
        else f"broadcast id={r[0]['task']['task_id'][:8]} "
        f"recipients={r[0]['notifications_sent_count']}"
    ),
    truncates_task_text=True,
)
def message_broadcast(ctx, agent_id, text, full):
    """Broadcast a message to all agents."""
    return broker.broadcast_message(
        ctx.obj["session_id"],
        agent_id,
        text,
    )


@message.command("poll")
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--since", default=None, hidden=True)
@click.option("--page-size", default=None, type=int, hidden=True)
@_full_flag_with_help("Disable body truncation.")
@click.pass_context
@_client_command(
    requires_agent_session=True,
    text_formatter=lambda r, *, full: output.format_indexed_list(
        r, lambda t: output.format_task(t, full=full), "No messages found."
    ),
    truncates_task_text=True,
)
def message_poll(ctx, agent_id, since, page_size, full):
    """Poll inbox for messages."""
    return broker.poll_tasks(
        agent_id,
        since=since,
        page_size=page_size,
    )


@message.command("ack")
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--task-id", required=True, help="Task ID to acknowledge")
@_full_flag_with_help("Disable body truncation.")
@_quiet_flag_with_help("Print only the task id.")
@click.pass_context
@_client_command(
    requires_agent_session=True,
    text_formatter=lambda r, *, full, quiet: (
        r["task"]["task_id"][:8]
        if quiet
        else "Message acknowledged.\n" + output.format_task(r, full=full)
    ),
    truncates_task_text=True,
)
def message_ack(ctx, agent_id, task_id, full, quiet):
    """Acknowledge receipt of a message."""
    return broker.ack_task(agent_id, task_id)


@message.command("cancel")
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--task-id", required=True, help="Task ID to cancel")
@_full_flag_with_help("Disable body truncation.")
@click.pass_context
@_client_command(
    requires_agent_session=True,
    text_formatter=lambda r, *, full: (
        "Task canceled.\n" + output.format_task(r, full=full)
    ),
    truncates_task_text=True,
)
def message_cancel(ctx, agent_id, task_id, full):
    """Cancel (retract) a sent message."""
    return broker.cancel_task(agent_id, task_id)


@message.command("show")
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--task-id", required=True, help="Task ID to retrieve")
@_full_flag_with_help("Disable body truncation.")
@click.pass_context
@_client_command(
    requires_agent_session=True,
    text_formatter=lambda r, *, full: output.format_task(r, full=full),
    truncates_task_text=True,
)
def message_show(ctx, agent_id, task_id, full):
    """Get details of a specific task."""
    return broker.get_task(ctx.obj["session_id"], task_id)


@agent.command("list")
@click.option("--agent-id", required=True, help="Agent ID")
@_full_flag
@click.pass_context
@_client_command(
    requires_agent_session=True,
    text_formatter=lambda agents, *, full: output.format_indexed_list(
        agents, lambda a: output.format_agent(a, full=full), "No agents found."
    ),
    renders_agent_card=True,
)
def agent_list(ctx, agent_id, full):
    """List registered agents in the session."""
    return broker.list_agents(ctx.obj["session_id"])


@agent.command("show")
@click.option("--agent-id", required=True, help="Agent ID")
@click.option("--id", "target_agent_id", required=True, help="Target agent ID")
@_full_flag
@click.pass_context
@_client_command(
    requires_agent_session=True,
    text_formatter=output.format_agent,
    renders_agent_card=True,
)
def agent_show(ctx, agent_id, target_agent_id, full):
    """Show detail for a specific agent."""
    result = broker.get_agent(target_agent_id, ctx.obj["session_id"])
    if result is None:
        raise click.ClickException(f"Agent {target_agent_id} not found")
    return result


@agent.command("deregister")
@click.option("--agent-id", required=True, help="Agent ID")
@click.pass_context
@_client_command(
    requires_agent_session=True,
    text_formatter=lambda _: "Agent deregistered successfully.",
)
def agent_deregister(ctx, agent_id):
    """Deregister this agent from the broker."""
    deregistered = broker.deregister_agent(agent_id)
    if not deregistered:
        raise click.ClickException(
            f"agent {agent_id} not found or already deregistered."
        )
    return {"status": "deregistered"}


@cli.group()
def member():
    """Manage tmux-backed member agents (Director only)."""


_PLACEMENT_MISSING_DEFAULT = (
    "agent {member_id} has no placement row; it was not "
    "spawned via `cafleet member create`."
)


def _require_member_pane(placement: dict, member_id: str, action: str) -> str:
    pane_id = placement["tmux_pane_id"]
    if pane_id is None:
        raise click.ClickException(
            f"member {member_id} has no pane yet (pending placement) "
            f"— nothing to {action}."
        )
    return pane_id


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
    (``cafleet agent deregister`` from delete; ``cafleet member create`` from
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


def _read_prompt_file(path: str) -> str:
    """Read the spawn prompt from a file, validating absolute path / readability / UTF-8 / non-empty.

    Owns the five error surfaces from design 0000059 § 6: relative path →
    ``UsageError``; missing / non-regular file → ``ClickException``;
    permission / generic-I/O failures → ``ClickException``; invalid UTF-8 →
    ``ClickException``; zero-byte or whitespace-only contents →
    ``ClickException``. Returns the file body verbatim — no stripping,
    trailing newlines preserved.

    The existence and regular-file checks ride entirely on the
    ``read_bytes()`` exception surface (no ``is_file()`` pre-check), so a
    path on which we lack traverse permission cannot leak through a stat
    gate and surface as the wrong message — ``PermissionError`` from
    ``read_bytes`` lands in the ``file is not readable`` branch directly.
    """
    if not Path(path).is_absolute():
        raise click.UsageError(
            f"--prompt-file requires an absolute path (got '{path}'). "
            "Resolve relative paths against your BASE first — "
            "see Skill(cafleet:base-dir)."
        )
    file_path = Path(path)
    try:
        # read_bytes() + decode() instead of read_text() so universal-newline
        # translation does NOT collapse CRLF / CR to LF — the success-criterion
        # promises the file body lands in the spawn argv byte-for-byte verbatim.
        content = file_path.read_bytes().decode("utf-8")
    except (FileNotFoundError, IsADirectoryError) as exc:
        # FileNotFoundError → the path does not name an existing file.
        # IsADirectoryError → the path names a directory, not a regular file.
        # Both map to the § 6 "missing or non-regular file" surface.
        raise click.ClickException(
            f"--prompt-file {path}: file does not exist or is not a regular file."
        ) from exc
    except PermissionError as exc:
        raise click.ClickException(
            f"--prompt-file {path}: file is not readable."
        ) from exc
    except UnicodeDecodeError as exc:
        raise click.ClickException(
            f"--prompt-file {path}: file is not valid UTF-8."
        ) from exc
    except OSError as exc:
        # Reserved for true I/O failures (EIO, ENOSPC, EBUSY, …). The earlier
        # FileNotFoundError / IsADirectoryError / PermissionError branches
        # handle the OSError subclasses that have a more precise § 6 message.
        raise click.ClickException(
            f"--prompt-file {path}: file is not readable."
        ) from exc
    if content == "" or content.isspace():
        raise click.ClickException(f"--prompt-file {path}: file is empty.")
    return content


def _resolve_prompt(
    ctx: click.Context,
    director_agent_id: str,
    new_agent_id: str,
    prompt_argv: tuple[str, ...],
    prompt_file: str | None = None,
) -> str:
    """Substitute ``session_id`` / ``agent_id`` / ``director_agent_id`` into the spawn prompt.

    Runs ``str.format`` on the chosen template (file > positional > default)
    so custom prompts must double literal braces (``{{`` / ``}}``) to survive
    the substitution.
    """
    session_id = ctx.obj["session_id"]
    if prompt_file is not None:
        template = _read_prompt_file(prompt_file)
    elif prompt_argv:
        template = " ".join(prompt_argv)
    else:
        template = _MEMBER_PROMPT_TEMPLATE
    try:
        return template.format(
            session_id=session_id,
            agent_id=new_agent_id,
            director_agent_id=director_agent_id,
        )
    except KeyError as exc:
        raise click.UsageError(
            f"Unknown placeholder {exc} in custom prompt. "
            "Supported placeholders: {session_id}, {agent_id}, "
            "{director_agent_id}. "
            "Double literal braces ({{, }}) to keep them as text."
        ) from exc
    except (ValueError, IndexError, AttributeError) as exc:
        raise click.UsageError(
            f"Malformed custom prompt: {exc}. "
            "Double literal braces ({{, }}) to keep them as text."
        ) from exc


def _deregister_with_warning(new_agent_id: str, *, session_id: str) -> None:
    """Best-effort deregister; emit warning to stderr if it fails."""
    try:
        broker.deregister_agent(new_agent_id)
    except Exception as drop_exc:
        click.echo(
            f"WARNING: rollback deregister failed — agent {new_agent_id} is "
            f"orphaned in the registry. Run `cafleet --session-id {session_id} "
            f"agent deregister --agent-id {new_agent_id}` manually to clean up. "
            f"Cause: {drop_exc}",
            err=True,
        )


def _rollback_register(new_agent_id: str, *, session_id: str, reason: str) -> NoReturn:
    """Best-effort deregister of a just-created agent, then raise ClickException."""
    _deregister_with_warning(new_agent_id, session_id=session_id)
    raise click.ClickException(f"{reason}. Rolled back registration of {new_agent_id}.")


@member.command("create")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.option("--name", required=True, help="Member name")
@click.option("--description", required=True, help="Member description")
@click.option(
    "--coding-agent",
    "coding_agent",
    type=click.Choice(["claude", "codex"]),
    default="claude",
    show_default=True,
    help="Coding agent (claude or codex).",
)
@click.option(
    "--prompt-file",
    "prompt_file",
    type=str,
    default=None,
    help="Read spawn prompt from FILE (abs path, UTF-8).",
)
@_full_flag
@click.argument("prompt_argv", nargs=-1)
@click.pass_context
def member_create(
    ctx, agent_id, name, description, coding_agent, prompt_file, full, prompt_argv
):
    """Register a new member and spawn its pane."""
    if prompt_file is not None and prompt_argv:
        raise click.UsageError(
            "--prompt-file and the positional prompt argument are mutually exclusive."
        )
    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]

    binary_name = _CLAUDE_BINARY if coding_agent == "claude" else _CODEX_BINARY

    try:
        tmux.ensure_tmux_available()
        _ensure_coding_agent_available(binary_name)
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
                "coding_agent": coding_agent,
            },
        )
    except Exception as exc:
        raise click.ClickException(f"register failed: {exc}") from exc
    new_agent_id = result["agent_id"]

    try:
        prompt = _resolve_prompt(ctx, agent_id, new_agent_id, prompt_argv, prompt_file)
    except (click.UsageError, click.ClickException):
        # Re-raise unwrapped so the exact message from docs/spec/cli-options.md
        # § Error Messages reaches the operator. Wrapping via _rollback_register
        # would prepend "prompt resolution failed:" and append "Rolled back
        # registration of <id>." (with a stray ".." when the inner message
        # already ends in a period), and would also downgrade UsageError exit
        # code 2 → ClickException exit 1.
        _deregister_with_warning(new_agent_id, session_id=session_id)
        raise

    if coding_agent == "claude":
        spawn_command = _build_claude_command(prompt, display_name=name)
    else:
        spawn_command = _build_codex_command(prompt)

    try:
        db_url = os.environ.get("CAFLEET_DATABASE_URL")
        fwd_env = {"CAFLEET_DATABASE_URL": db_url} if db_url else {}
        pane_id = tmux.split_window(
            target_window_id=director_ctx.window_id,
            env=fwd_env,
            command=spawn_command,
        )
    except tmux.TmuxError as exc:
        _rollback_register(
            new_agent_id,
            session_id=session_id,
            reason=f"tmux split-window failed: {exc}",
        )

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
    if placement_view is None:
        with contextlib.suppress(tmux.TmuxError):
            tmux.send_exit(target_pane_id=pane_id, ignore_missing=True)
        _rollback_register(
            new_agent_id,
            session_id=session_id,
            reason="placement row vanished before pane-id patch",
        )

    try:
        tmux.select_layout(target_window_id=director_ctx.window_id)
    except tmux.TmuxError as exc:
        click.echo(f"Warning: select-layout failed: {exc}", err=True)

    result["placement"] = placement_view
    if ctx.obj["json_output"]:
        click.echo(output.format_json(result, pretty=ctx.obj["pretty"]))
    else:
        click.echo(output.format_member(result, full=full))


@member.command("delete")
@_director_member_options
@click.option(
    "--force",
    "-f",
    "force",
    is_flag=True,
    default=False,
    help="Skip /exit; kill-pane immediately.",
)
@click.pass_context
def member_delete(ctx, agent_id, member_id, force):
    """Deregister a member agent and close its tmux pane."""
    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]

    _ensure_tmux_or_die()

    _target, placement = _load_authorized_member(
        session_id,
        agent_id,
        member_id,
        placement_missing_msg=(
            f"agent {member_id} has no placement; use `cafleet agent deregister` instead"
        ),
    )
    pane_id = placement["tmux_pane_id"]

    if pane_id is None:
        try:
            broker.deregister_agent(member_id)
        except Exception as exc:
            raise click.ClickException(f"deregister failed: {exc}") from exc
        pane_status = "(pending — no pane)"
        _emit_member_delete_output(
            ctx, member_id, pane_status, header="Member deleted."
        )
        return

    if force:
        try:
            tmux.kill_pane(target_pane_id=pane_id, ignore_missing=True)
        except tmux.TmuxError as exc:
            raise click.ClickException(
                f"kill_pane failed for pane {pane_id}: {exc}. "
                f"The tmux server may be unreachable. Verify with 'cafleet doctor', "
                f"then re-run the command."
            ) from exc
        try:
            broker.deregister_agent(member_id)
        except Exception as exc:
            raise click.ClickException(f"deregister failed: {exc}") from exc
        try:
            tmux.select_layout(target_window_id=placement["tmux_window_id"])
        except tmux.TmuxError as exc:
            click.echo(f"Warning: select-layout failed: {exc}", err=True)
        pane_status = f"{pane_id} (killed)"
        _emit_member_delete_output(
            ctx, member_id, pane_status, header="Member deleted (--force)."
        )
        return

    try:
        tmux.send_exit(target_pane_id=pane_id, ignore_missing=True)
    except tmux.TmuxError as exc:
        raise click.ClickException(
            f"send_exit failed for pane {pane_id}: {exc}. "
            f"The tmux server may be unreachable. Verify with 'cafleet doctor', "
            f"then re-run 'cafleet member delete', or use '--force' to kill the "
            f"pane directly."
        ) from exc

    try:
        gone = tmux.wait_for_pane_gone(
            target_pane_id=pane_id, timeout=15.0, interval=0.5
        )
    except tmux.TmuxError as exc:
        raise click.ClickException(
            f"tmux call failed while waiting for pane {pane_id} to close: {exc}"
        ) from exc

    if gone:
        try:
            broker.deregister_agent(member_id)
        except Exception as exc:
            raise click.ClickException(f"deregister failed: {exc}") from exc
        try:
            tmux.select_layout(target_window_id=placement["tmux_window_id"])
        except tmux.TmuxError as exc:
            click.echo(f"Warning: select-layout failed: {exc}", err=True)
        pane_status = f"{pane_id} (closed)"
        _emit_member_delete_output(
            ctx, member_id, pane_status, header="Member deleted."
        )
        return

    try:
        tail = tmux.capture_pane(target_pane_id=pane_id, lines=80)
    except tmux.TmuxError as exc:
        click.echo(
            f"Warning: capture_pane failed during timeout handling: {exc}. "
            f"The timeout error and recovery hint still print.",
            err=True,
        )
        tail = ""

    click.echo(
        f"Error: pane {pane_id} did not close within 15.0s after /exit.", err=True
    )
    click.echo(f"--- pane {pane_id} tail (last 80 lines) ---", err=True)
    click.echo(tail, err=True)
    click.echo("---", err=True)
    click.echo(
        "Recovery: inspect with `cafleet member capture`, answer any prompt with "
        "`cafleet member send-input`, then re-run `cafleet member delete`. "
        "Or re-run with `--force` to skip the wait and kill the pane.",
        err=True,
    )

    pane_status = f"{pane_id} (timeout)"
    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {"agent_id": member_id, "pane_status": pane_status},
                pretty=ctx.obj["pretty"],
            )
        )
    ctx.exit(2)


def _emit_member_delete_output(
    ctx: click.Context,
    member_id: str,
    pane_status: str,
    *,
    header: str,
) -> None:
    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {"agent_id": member_id, "pane_status": pane_status},
                pretty=ctx.obj["pretty"],
            )
        )
    else:
        click.echo(header)
        click.echo(f"  agent_id:  {member_id}")
        click.echo(f"  pane_id:   {pane_status}")


@member.command("list")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.option(
    "--activity",
    "activity",
    is_flag=True,
    default=False,
    hidden=True,
)
@click.pass_context
def member_list(ctx, agent_id, activity):
    """List member agents managed by this Director."""
    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]
    try:
        if activity:
            rows = broker.list_members_with_activity(session_id, agent_id)
        else:
            rows = broker.list_members(session_id, agent_id)
    except click.ClickException:
        raise
    except Exception as exc:
        raise click.ClickException(str(exc)) from exc
    if ctx.obj["json_output"]:
        click.echo(output.format_json(rows, pretty=ctx.obj["pretty"]))
    elif activity:
        click.echo(output.format_member_list_activity(rows))
    else:
        click.echo(output.format_member_list(rows))


@member.command("capture")
@_director_member_options
@click.option(
    "--lines",
    "--tail",
    "lines",
    type=int,
    default=30,
    show_default=True,
    help="Lines to capture (alias: --tail).",
)
@click.option(
    "--ansi/--no-ansi",
    default=False,
    hidden=True,
)
@click.pass_context
def member_capture(ctx, agent_id, member_id, lines, ansi):
    """Capture the last N lines of a member pane's terminal buffer."""
    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]

    _ensure_tmux_or_die()

    _target, placement = _load_authorized_member(
        session_id,
        agent_id,
        member_id,
        placement_missing_msg=_PLACEMENT_MISSING_DEFAULT.format(member_id=member_id),
    )
    pane_id = _require_member_pane(placement, member_id, "capture")

    try:
        content = tmux.capture_pane(target_pane_id=pane_id, lines=lines)
    except tmux.TmuxError as exc:
        raise click.ClickException(f"capture failed: {exc}") from exc

    if not ansi:
        content = output.strip_ansi(content)

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "member_agent_id": member_id,
                    "pane_id": pane_id,
                    "lines": lines,
                    "content": content,
                },
                pretty=ctx.obj["pretty"],
            )
        )
    else:
        # color=True preserves ANSI escape sequences on non-TTY sinks (e.g.
        # CliRunner-captured stdout). Without it, click.echo would re-strip
        # the escapes the operator just opted into via --ansi.
        click.echo(content, nl=False, color=True if ansi else None)


@member.command("send-input")
@_director_member_options
@click.option(
    "--choice",
    type=click.IntRange(1, 3),
    default=None,
    help="Choice 1/2/3 (xor --freetext).",
)
@click.option(
    "--freetext",
    type=str,
    default=None,
    hidden=True,
)
@click.pass_context
def member_send_input(ctx, agent_id, member_id, choice, freetext):
    """Safely forward a restricted keystroke to a member pane."""
    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]

    if freetext is not None and freetext.lstrip().startswith("!"):
        raise click.UsageError(
            "--freetext may not start with '!' — that triggers the coding agent's "
            "shell-execution shortcut. Use 'cafleet member exec' for shell dispatch instead."
        )

    supplied = sum(1 for v in (choice, freetext) if v is not None)
    if supplied != 1:
        raise click.UsageError(
            "--choice and --freetext are mutually exclusive; supply exactly one."
        )

    if freetext is not None and ("\n" in freetext or "\r" in freetext):
        raise click.UsageError("free text may not contain newlines.")

    _ensure_tmux_or_die()

    target, placement = _load_authorized_member(
        session_id,
        agent_id,
        member_id,
        placement_missing_msg=_PLACEMENT_MISSING_DEFAULT.format(member_id=member_id),
    )
    pane_id = _require_member_pane(placement, member_id, "send")

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
                },
                pretty=ctx.obj["pretty"],
            )
        )
    else:
        label = f"choice {value}" if action == "choice" else "free text"
        click.echo(f"Sent {label} to member {target['name']} ({pane_id}).")


@member.command("exec")
@_director_member_options
@click.argument("command")
@click.pass_context
def member_exec(ctx, agent_id, member_id, command):
    """Dispatch a shell command via the coding agent's `!` shortcut."""
    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]

    if "\n" in command or "\r" in command:
        raise click.UsageError("command may not contain newlines.")
    if not command.strip():
        raise click.UsageError("command may not be empty.")
    command = command.strip()

    _ensure_tmux_or_die()

    target, placement = _load_authorized_member(
        session_id,
        agent_id,
        member_id,
        placement_missing_msg=_PLACEMENT_MISSING_DEFAULT.format(member_id=member_id),
    )
    pane_id = _require_member_pane(placement, member_id, "send")

    try:
        tmux.send_bash_command(target_pane_id=pane_id, command=command)
    except tmux.TmuxError as exc:
        raise click.ClickException(f"send failed: {exc}") from exc

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "member_agent_id": member_id,
                    "pane_id": pane_id,
                    "command": command,
                },
                pretty=ctx.obj["pretty"],
            )
        )
    else:
        click.echo(
            f"Sent bash command {command!r} to member {target['name']} ({pane_id})."
        )


@member.command("ping")
@_director_member_options
@_quiet_flag
@click.pass_context
def member_ping(ctx, agent_id, member_id, quiet):
    """Inject an inbox-poll keystroke into a member's pane (Director-only)."""
    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]

    _ensure_tmux_or_die()

    target, placement = _load_authorized_member(
        session_id,
        agent_id,
        member_id,
        placement_missing_msg=_PLACEMENT_MISSING_DEFAULT.format(member_id=member_id),
    )
    pane_id = _require_member_pane(placement, member_id, "send")

    try:
        ok = tmux.send_poll_trigger(
            target_pane_id=pane_id,
            session_id=session_id,
            agent_id=member_id,
        )
    except tmux.TmuxError as exc:
        raise click.ClickException(f"send failed: {exc}") from exc
    if not ok:
        raise click.ClickException(
            f"send failed: tmux send-keys did not deliver the poll-trigger "
            f"keystroke to pane {pane_id}."
        )

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "member_agent_id": member_id,
                    "pane_id": pane_id,
                },
                pretty=ctx.obj["pretty"],
            )
        )
    elif quiet:
        click.echo(member_id[:8])
    else:
        click.echo(
            f"Pinged member {target['name']} ({pane_id}) — poll keystroke dispatched."
        )


if __name__ == "__main__":
    cli()
