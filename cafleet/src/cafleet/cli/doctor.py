"""``cafleet doctor`` — tmux pane-identity and skills-install diagnostics."""

import importlib.metadata
import os

import click

from cafleet import output
from cafleet.broker.skill_installs import (
    list_skill_installs,
    skill_installs_table_exists,
)
from cafleet.cli._helpers import ensure_tmux_or_die
from cafleet.multiplexer import MULTIPLEXERS, TmuxError


def _skills_report() -> dict:
    cli_version = importlib.metadata.version("cafleet")
    installs = []
    if skill_installs_table_exists():
        installs = [
            {**row, "current": row["cafleet_version"] == cli_version}
            for row in list_skill_installs()
        ]
    return {"cli_version": cli_version, "installs": installs}


@click.command("doctor")
@click.pass_context
def doctor(ctx) -> None:
    """Print the calling pane's tmux identifiers and the skills-install report."""
    ensure_tmux_or_die()

    try:
        director_ctx = MULTIPLEXERS["tmux"].context_discovery()
    except TmuxError as exc:
        raise click.ClickException(str(exc)) from exc

    tmux_pane_env = os.environ["TMUX_PANE"]
    skills = _skills_report()

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "tmux": {
                        "session_name": director_ctx.session,
                        "window_id": director_ctx.window_id,
                        "pane_id": director_ctx.pane_id,
                        "tmux_pane_env": tmux_pane_env,
                    },
                    "skills": skills,
                },
            )
        )
    else:
        click.echo("tmux:")
        click.echo(f"  session_name:  {director_ctx.session}")
        click.echo(f"  window_id:     {director_ctx.window_id}")
        click.echo(f"  pane_id:       {director_ctx.pane_id}")
        click.echo(f"  TMUX_PANE:     {tmux_pane_env}")
        click.echo("skills:")
        if not skills["installs"]:
            click.echo("  (no skills install recorded; run 'cafleet setup')")
        else:
            click.echo(f"  {'cli_version:':<13}{skills['cli_version']}")
            for row in skills["installs"]:
                verdict = "ok" if row["current"] else "STALE"
                click.echo(
                    f"  {row['coding_agent'] + ':':<13}{row['cafleet_version']} "
                    f"({row['installed_at']}) {verdict}"
                )
