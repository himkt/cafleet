"""Shared decorators and guards for CLI subcommands."""

import importlib.metadata

import click

from cafleet.broker.asset_installs import (
    asset_installs_table_exists,
    list_asset_installs,
)
from cafleet.multiplexer import Multiplexer, MultiplexerError, resolve_multiplexer


def ensure_multiplexer_or_die() -> Multiplexer:
    try:
        mux = resolve_multiplexer()
        mux.ensure_available()
    except MultiplexerError as exc:
        raise click.ClickException(str(exc)) from exc
    return mux


def ensure_assets_current() -> None:
    """Hard-error when no assets install is recorded or any recorded one is stale."""
    rows = list_asset_installs() if asset_installs_table_exists() else []
    if not rows:
        raise click.ClickException(
            "no assets install is recorded; run 'cafleet setup' first"
        )
    runtime_version = importlib.metadata.version("cafleet")
    stale = [row for row in rows if row["cafleet_version"] != runtime_version]
    if stale:
        listed = ", ".join(
            f"{row['coding_agent']}={row['cafleet_version']}" for row in stale
        )
        raise click.ClickException(
            f"stale assets detected ({listed}; CLI {runtime_version}); "
            "run 'cafleet setup' to reinstall"
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
json_flag = click.option(
    "--json",
    "json_output",
    is_flag=True,
    default=False,
    help="Output in JSON format.",
)


member_id_option = click.option(
    "--member-id",
    "member_id",
    type=int,
    required=True,
    help="Member ID (the member in question)",
)
from_member_id_option = click.option(
    "--from-member-id",
    "from_member_id",
    type=int,
    required=True,
    help="Sender's member ID",
)
to_member_id_option = click.option(
    "--to-member-id",
    "to_member_id",
    type=int,
    required=True,
    help="Recipient member ID",
)


def text_body_options(text_help: str):
    """Apply the shared ``--text`` / ``--text-file`` option pair.

    ``--text`` carries the per-command ``text_help``; ``--text-file`` is uniform.
    ``--text`` is applied last (outermost) so it precedes ``--text-file`` in
    ``--help``.
    """

    def decorator(func):
        func = click.option(
            "--text-file",
            "text_file",
            default=None,
            help="File (UTF-8) or '-' for stdin.",
        )(func)
        return click.option("--text", "text", default=None, help=text_help)(func)

    return decorator


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
