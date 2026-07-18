"""``cafleet doctor`` — multiplexer pane-identity and assets-install diagnostics."""

import importlib.metadata
import os

import click

from cafleet import output
from cafleet.broker.asset_installs import (
    asset_installs_table_exists,
    list_asset_installs,
)
from cafleet.cli._helpers import ensure_multiplexer_or_die, json_flag
from cafleet.multiplexer import MultiplexerError

_PRESENCE_ENV = {"tmux": "TMUX", "herdr": "HERDR_ENV"}


def _assets_report() -> dict:
    cli_version = importlib.metadata.version("cafleet")
    installs = []
    if asset_installs_table_exists():
        installs = [
            {**row, "current": row["cafleet_version"] == cli_version}
            for row in list_asset_installs()
        ]
    return {"cli_version": cli_version, "installs": installs}


@click.command("doctor")
@json_flag
def doctor(json_output: bool) -> None:
    """Print the resolved multiplexer backend, pane identifiers, and assets report."""
    mux = ensure_multiplexer_or_die()

    try:
        pane_ctx = mux.context_discovery()
    except MultiplexerError as exc:
        raise click.ClickException(str(exc)) from exc

    presence_var = _PRESENCE_ENV[mux.name]
    presence_value = os.environ.get(presence_var, "")
    assets = _assets_report()

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
                    "assets": assets,
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
        click.echo("assets:")
        if not assets["installs"]:
            click.echo("  (no assets install recorded; run 'cafleet setup')")
        else:
            click.echo(f"  {'cli_version:':<13}{assets['cli_version']}")
            for row in assets["installs"]:
                verdict = "ok" if row["current"] else "STALE"
                click.echo(
                    f"  {row['coding_agent'] + ':':<13}{row['cafleet_version']} "
                    f"({row['installed_at']}) {verdict}"
                )
