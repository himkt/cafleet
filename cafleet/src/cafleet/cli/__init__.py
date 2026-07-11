"""cafleet CLI."""

import click

from cafleet.cli.doctor import doctor
from cafleet.cli.fleet import fleet
from cafleet.cli.member import member
from cafleet.cli.message import message
from cafleet.cli.monitor import monitor
from cafleet.cli.server import server
from cafleet.cli.setup import setup as setup_command


@click.group()
@click.version_option(package_name="cafleet", message="cafleet %(version)s")
@click.option(
    "--json", "json_output", is_flag=True, default=False, help="Output in JSON format"
)
@click.pass_context
def cli(ctx, json_output):
    """CAFleet — CLI for the message broker and member registry."""
    ctx.ensure_object(dict)
    ctx.obj["json_output"] = json_output


cli.add_command(fleet)
cli.add_command(message)
cli.add_command(member)
cli.add_command(monitor)
cli.add_command(server)
cli.add_command(doctor)
cli.add_command(setup_command)
