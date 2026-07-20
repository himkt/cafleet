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
def cli() -> None:
    """CAFleet — CLI for the message broker and member registry."""


cli.add_command(fleet)
cli.add_command(message)
cli.add_command(member)
cli.add_command(monitor)
cli.add_command(server)
cli.add_command(doctor)
cli.add_command(setup_command)
