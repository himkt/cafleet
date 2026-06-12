"""``cafleet doctor`` — tmux pane-identity diagnostics."""

import os

import click

from cafleet import output
from cafleet.cli._helpers import ensure_tmux_or_die
from cafleet.multiplexer import MULTIPLEXERS, TmuxError


@click.command("doctor")
@click.pass_context
def doctor(ctx) -> None:
    """Print the calling pane's tmux session/window/pane identifiers."""
    ensure_tmux_or_die()

    try:
        director_ctx = MULTIPLEXERS["tmux"].context_discovery()
    except TmuxError as exc:
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
            )
        )
    else:
        click.echo("tmux:")
        click.echo(f"  session_name:  {director_ctx.session}")
        click.echo(f"  window_id:     {director_ctx.window_id}")
        click.echo(f"  pane_id:       {director_ctx.pane_id}")
        click.echo(f"  TMUX_PANE:     {tmux_pane_env}")
