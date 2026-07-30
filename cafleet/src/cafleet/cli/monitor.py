"""``cafleet monitor`` — the monitoring toolkit: the loop and its read primitive.

``start`` runs the `scan → wake → sleep` loop in-process — a coding agent
launches it as a background task and owns its lifetime (there is no detached
process and no ``stop`` command; stop the background task to stop it).
``capture`` reads the tail of a member pane's terminal buffer. The loop is
reached through a module attribute (``loop.run_monitor_loop``) — the
established broker-style indirection.
"""

import hashlib
from datetime import UTC, datetime

import click

from cafleet import broker, output
from cafleet.cli._helpers import (
    ensure_assets_current,
    ensure_multiplexer_or_die,
    fleet_id_option,
    json_flag,
    member_id_option,
)
from cafleet.cli.member import _load_authorized_member, _require_member_pane
from cafleet.monitor import DEFAULT_TICK_SECONDS, loop
from cafleet.multiplexer import MultiplexerError


@click.group()
def monitor() -> None:
    """Supervision scheduler (heartbeat) commands."""
    ensure_assets_current()


def _require_live_fleet(fleet_id: int) -> None:
    fleet = broker.get_fleet(fleet_id)
    if fleet is None or fleet["deleted_at"] is not None:
        raise click.ClickException(f"fleet {fleet_id} not found")


@monitor.command("start")
@fleet_id_option
@click.option(
    "--tick",
    type=click.IntRange(min=1),
    default=DEFAULT_TICK_SECONDS,
    show_default=True,
    help="Scan-tick cadence in seconds.",
)
@click.pass_context
def monitor_start(ctx: click.Context, tick: int) -> None:
    """Run the fleet's monitor loop in-process (launch as a background task).

    The loop blocks until signalled (SIGTERM/SIGINT) or the fleet is torn down.
    A coding agent launches it as a background task and stops it by stopping
    that task — there is no detached process and no ``stop`` command.
    """
    fleet_id = ctx.obj["fleet_id"]
    _require_live_fleet(fleet_id)
    ensure_multiplexer_or_die()
    if broker.find_monitoring_member(fleet_id) is None:
        click.echo(
            f"Warning: fleet {fleet_id} has no monitoring member; the "
            f"monitor heartbeat will wake no member. Spawn one first with "
            f"'cafleet member create --role monitor'.",
            err=True,
        )
    loop.run_monitor_loop(fleet_id, tick)


@monitor.command("capture")
@fleet_id_option
@member_id_option
@click.option(
    "--lines",
    "lines",
    type=int,
    default=20,
    show_default=True,
    help="Lines to capture.",
)
@click.option(
    "--ansi/--no-ansi",
    default=False,
    help="Emit raw ANSI instead of stripping it.",
)
@json_flag
@click.pass_context
def monitor_capture(ctx, member_id, lines, ansi, json_output):
    """Capture the last N lines of a member pane's terminal buffer."""
    fleet_id = ctx.obj["fleet_id"]

    mux = ensure_multiplexer_or_die()

    target, placement = _load_authorized_member(
        fleet_id,
        member_id,
    )
    member_id = target["member_id"]
    pane_id = _require_member_pane(placement, member_id, "capture")

    try:
        content = mux.capture_pane(target_pane_id=pane_id, lines=lines)
    except MultiplexerError as exc:
        raise click.ClickException(f"capture failed: {exc}") from exc
    captured_at = datetime.now(UTC).isoformat()

    if not ansi:
        content = output.strip_ansi(content)

    if json_output:
        click.echo(
            output.format_json(
                {
                    "member_id": member_id,
                    "pane_id": pane_id,
                    "lines": lines,
                    "content": content,
                    "captured_at": captured_at,
                    "content_sha256": hashlib.sha256(
                        content.encode("utf-8")
                    ).hexdigest(),
                },
            )
        )
    else:
        # color=True preserves ANSI escape sequences on non-TTY sinks (e.g.
        # CliRunner-captured stdout). Without it, click.echo would re-strip
        # the escapes the operator just opted into via --ansi.
        click.echo(content, nl=False, color=True if ansi else None)
