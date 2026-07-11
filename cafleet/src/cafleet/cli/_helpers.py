"""Shared decorators and guards for CLI subcommands."""

import functools
import importlib.metadata
from collections.abc import Callable

import click

from cafleet import broker, output
from cafleet.broker.skill_installs import (
    list_skill_installs,
    skill_installs_table_exists,
)
from cafleet.multiplexer import Multiplexer, MultiplexerError, resolve_multiplexer


def ensure_multiplexer_or_die() -> Multiplexer:
    try:
        mux = resolve_multiplexer()
        mux.ensure_available()
    except MultiplexerError as exc:
        raise click.ClickException(str(exc)) from exc
    return mux


def ensure_skills_current() -> None:
    """Hard-error when no skills install is recorded or any recorded one is stale."""
    rows = list_skill_installs() if skill_installs_table_exists() else []
    if not rows:
        raise click.ClickException(
            "no skills install is recorded; run 'cafleet setup' first"
        )
    runtime_version = importlib.metadata.version("cafleet")
    stale = [row for row in rows if row["cafleet_version"] != runtime_version]
    if stale:
        listed = ", ".join(
            f"{row['coding_agent']}={row['cafleet_version']}" for row in stale
        )
        raise click.ClickException(
            f"stale skills detected ({listed}; CLI {runtime_version}); "
            "run 'cafleet setup skill' to reinstall"
        )


full_flag = click.option(
    "--full",
    "full",
    is_flag=True,
    default=False,
    help="Render the full, untruncated output.",
)
quiet_flag = click.option(
    "--quiet",
    "quiet",
    is_flag=True,
    default=False,
    help="Print only the resulting id.",
)


member_id_option = click.option(
    "--member-id",
    "member_id",
    type=int,
    required=True,
    help="Member ID (the member in question)",
)
from_member_id_option = click.option(
    "--from-member-id",
    "from_member_id",
    type=int,
    required=True,
    help="Sender's member ID",
)
to_member_id_option = click.option(
    "--to-member-id",
    "to_member_id",
    type=int,
    required=True,
    help="Recipient member ID",
)


def text_body_options(text_help: str):
    """Apply the shared ``--text`` / ``--text-file`` option pair.

    ``--text`` carries the per-command ``text_help``; ``--text-file`` is uniform.
    ``--text`` is applied last (outermost) so it precedes ``--text-file`` in
    ``--help``.
    """

    def decorator(func):
        func = click.option(
            "--text-file",
            "text_file",
            default=None,
            help="File (UTF-8) or '-' for stdin.",
        )(func)
        return click.option("--text", "text", default=None, help=text_help)(func)

    return decorator


def _fleet_id_callback(
    ctx: click.Context, param: click.Parameter, value: int | None
) -> int:
    if value is None:
        raise click.ClickException(
            "--fleet-id <int> is required for this subcommand. "
            "Create a fleet with 'cafleet fleet create' and pass its id."
        )
    ctx.ensure_object(dict)
    ctx.obj["fleet_id"] = value
    return value


fleet_id_option = click.option(
    "--fleet-id",
    "fleet_id",
    type=int,
    default=None,
    callback=_fleet_id_callback,
    expose_value=False,
    help="Fleet ID (integer); required for this subcommand.",
)


def client_command(
    *,
    requires_member_fleet: bool = False,
    member_kwarg: str = "member_id",
    text_formatter: Callable[..., str],
):
    """Subsume the boilerplate blocks shared by the ``message`` subcommands.

    ``member_kwarg`` names the acting-member kwarg the fleet gate reads —
    ``member_id`` for the single-member commands, ``from_member_id`` for the
    two-party senders. JSON output goes through ``render_tasks_in_result`` +
    ``truncate_task_text``; the text formatter is called as
    ``text_formatter(result, full=, quiet=)``.
    """

    def decorator(func):
        @functools.wraps(func)
        def wrapper(ctx, *args, **kwargs):
            fleet_id = ctx.obj["fleet_id"]
            try:
                if requires_member_fleet:
                    member_id = kwargs.get(member_kwarg)
                    if member_id is None:
                        raise click.ClickException(
                            f"client_command(requires_member_fleet=True) but no "
                            f"'{member_kwarg}' kwarg was passed. Check the "
                            f"@click.option declaration on this command."
                        )
                    if not broker.verify_member_fleet(member_id, fleet_id):
                        raise click.ClickException(
                            f"member {member_id} is not in fleet {fleet_id}."
                        )
                result = func(ctx, *args, **kwargs)
                full = kwargs["full"]
                output.truncate_task_text(result, full=full)
                rendered = output.render_tasks_in_result(result, full=full)
                if ctx.obj["json_output"]:
                    click.echo(output.format_json(rendered))
                else:
                    extra = {"quiet": kwargs["quiet"]} if "quiet" in kwargs else {}
                    click.echo(text_formatter(result, full=full, **extra))
            except click.ClickException:
                raise
            except Exception as exc:
                raise click.ClickException(str(exc)) from exc
            return result

        return wrapper

    return decorator
