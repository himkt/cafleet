"""``cafleet doctor`` — multiplexer pane-identity and skills-install diagnostics."""

import importlib.metadata
import os

import click

from cafleet import output
from cafleet.broker.skill_installs import (
    list_skill_installs,
    skill_installs_table_exists,
)
from cafleet.cli._helpers import ensure_multiplexer_or_die, json_flag
from cafleet.multiplexer import MultiplexerError

_PRESENCE_ENV = {"tmux": "TMUX", "herdr": "HERDR_ENV"}


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
@json_flag
def doctor(json_output: bool) -> None:
    """Print the resolved multiplexer backend, pane identifiers, and skills report."""
    mux = ensure_multiplexer_or_die()

    try:
        pane_ctx = mux.context_discovery()
    except MultiplexerError as exc:
        raise click.ClickException(str(exc)) from exc

    presence_var = _PRESENCE_ENV[mux.name]
    presence_value = os.environ.get(presence_var, "")
    skills = _skills_report()

    if json_output:
        click.echo(
            output.format_json(
                {
                    "multiplexer": {
                        "backend": mux.name,
                        "session": pane_ctx.session,
                        "window_id": pane_ctx.window_id,
                        "pane_id": pane_ctx.pane_id,
                        "presence_var": presence_var,
                        "presence_value": presence_value,
                    },
                    "skills": skills,
                },
            )
        )
    else:
        click.echo("multiplexer:")
        click.echo(f"  backend:       {mux.name}")
        click.echo(f"  session:       {pane_ctx.session}")
        click.echo(f"  window_id:     {pane_ctx.window_id}")
        click.echo(f"  pane_id:       {pane_ctx.pane_id}")
        click.echo(f"  presence:      {presence_var}={presence_value}")
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
