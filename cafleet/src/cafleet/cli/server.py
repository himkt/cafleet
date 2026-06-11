"""``cafleet server`` — admin WebUI server launcher."""

import click

from cafleet.config import settings


@click.command("server")
@click.option(
    "--host",
    default=settings.broker_host,
    show_default=True,
    help="Bind address.",
)
@click.option(
    "--port",
    default=settings.broker_port,
    show_default=True,
    type=int,
    help="Bind port.",
)
def server(host: str, port: int) -> None:
    """Start the admin WebUI FastAPI server."""
    import uvicorn

    uvicorn.run(
        "cafleet.webui.app:app",
        host=host,
        port=port,
    )
