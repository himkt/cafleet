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
from cafleet.multiplexer import MULTIPLEXERS, TmuxError


def ensure_tmux_or_die() -> None:
    try:
        MULTIPLEXERS["tmux"].ensure_available()
    except TmuxError as exc:
        raise click.ClickException(str(exc)) from exc


def ensure_skills_current() -> None:
    """Hard-error when no skills install is recorded or any recorded one is stale."""
    if not skill_installs_table_exists():
        raise click.ClickException(
            "no skills install is recorded; run 'cafleet setup' first"
        )
    rows = list_skill_installs()
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


def director_member_options(func):
    return click.option(
        "--member-id", type=int, required=True, help="Target member's agent ID"
    )(func)


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
    requires_agent_fleet: bool = False,
    text_formatter: Callable[..., str] | None = None,
    truncates_task_text: bool = False,
):
    """Subsume the boilerplate blocks shared by the ``message`` subcommands.

    Branches:
    - ``truncates_task_text=True`` → JSON output goes through
      ``render_tasks_in_result`` + ``truncate_task_text``; text formatter is
      called as ``text_formatter(result, full=, quiet=)``.
    - Otherwise → JSON output is the raw broker result; text formatter is
      called as ``text_formatter(result)``.
    """

    def decorator(func):
        @functools.wraps(func)
        def wrapper(ctx, *args, **kwargs):
            fleet_id = ctx.obj["fleet_id"]
            try:
                if requires_agent_fleet:
                    agent_id = kwargs.get("agent_id")
                    if agent_id is None:
                        raise click.ClickException(
                            "client_command(requires_agent_fleet=True) but no "
                            "'agent_id' kwarg was passed. Check the @click.option "
                            "declaration on this command."
                        )
                    if not broker.verify_agent_fleet(agent_id, fleet_id):
                        raise click.ClickException(
                            f"agent {agent_id} is not a member of fleet {fleet_id}."
                        )
                result = func(ctx, *args, **kwargs)
                full = kwargs.get("full", False)
                if truncates_task_text:
                    output.truncate_task_text(result, full=full)
                    rendered = output.render_tasks_in_result(result, full=full)
                else:
                    rendered = result
                if ctx.obj["json_output"]:
                    click.echo(output.format_json(rendered))
                elif text_formatter is not None:
                    if truncates_task_text:
                        extra = {"quiet": kwargs["quiet"]} if "quiet" in kwargs else {}
                        click.echo(text_formatter(result, full=full, **extra))
                    else:
                        click.echo(text_formatter(result))
                else:
                    click.echo(output.format_json(rendered))
            except click.ClickException:
                raise
            except Exception as exc:
                raise click.ClickException(str(exc)) from exc
            return result

        return wrapper

    return decorator
