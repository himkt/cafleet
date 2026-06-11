"""cafleet CLI."""

import click

from cafleet.cli.agent import agent
from cafleet.cli.db import db
from cafleet.cli.doctor import doctor
from cafleet.cli.fleet import fleet
from cafleet.cli.member import member
from cafleet.cli.message import message
from cafleet.cli.server import server


@click.group()
@click.version_option(package_name="cafleet", message="cafleet %(version)s")
@click.option(
    "--json", "json_output", is_flag=True, default=False, help="Output in JSON format"
)
@click.option(
    "--fleet-id",
    "fleet_id",
    type=int,
    default=None,
    help="Fleet ID (integer); required for client subcommands.",
)
@click.pass_context
def cli(ctx, json_output, fleet_id):
    """CAFleet — CLI for the message broker and agent registry."""
    ctx.ensure_object(dict)
    ctx.obj["fleet_id"] = fleet_id
    ctx.obj["json_output"] = json_output


cli.add_command(db)
cli.add_command(fleet)
cli.add_command(agent)
cli.add_command(message)
cli.add_command(member)
cli.add_command(server)
cli.add_command(doctor)
