"""Shared decorators and guards for CLI subcommands."""

import functools
from collections.abc import Callable

import click

from cafleet import broker, output
from cafleet.multiplexer import MULTIPLEXERS, TmuxError


def ensure_tmux_or_die() -> None:
    try:
        MULTIPLEXERS["tmux"].ensure_available()
    except TmuxError as exc:
        raise click.ClickException(str(exc)) from exc


full_flag = click.option("--full", "full", is_flag=True, default=False, hidden=True)
quiet_flag = click.option("--quiet", "quiet", is_flag=True, default=False, hidden=True)


def director_member_options(func):
    return click.option(
        "--member-id", type=int, required=True, help="Target member's agent ID"
    )(func)


def require_fleet_id(ctx: click.Context) -> None:
    if ctx.obj["fleet_id"] is None:
        raise click.ClickException(
            "--fleet-id <int> is required for this subcommand. "
            "Create a fleet with 'cafleet fleet create' and pass its id."
        )


def client_command(
    *,
    requires_agent_fleet: bool = False,
    text_formatter: Callable[..., str] | None = None,
    truncates_task_text: bool = False,
    renders_agent_card: bool = False,
):
    """Subsume the boilerplate blocks shared by client subcommands.

    Branches:
    - ``truncates_task_text=True`` → JSON output goes through
      ``render_tasks_in_result`` + ``truncate_task_text``; text formatter is
      called as ``text_formatter(result, full=, quiet=)``.
    - ``renders_agent_card=True`` → JSON output goes through
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
            require_fleet_id(ctx)
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
                elif renders_agent_card:
                    rendered = output.render_agents_in_result(result, full=full)
                else:
                    rendered = result
                if ctx.obj["json_output"]:
                    click.echo(output.format_json(rendered))
                elif text_formatter is not None:
                    if truncates_task_text:
                        extra = {"quiet": kwargs["quiet"]} if "quiet" in kwargs else {}
                        click.echo(text_formatter(result, full=full, **extra))
                    elif renders_agent_card:
                        click.echo(text_formatter(result, full=full))
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
