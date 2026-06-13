"""``cafleet monitor`` — supervision-scheduler lifecycle + schedule commands.

``start`` / ``stop`` manage the detached per-fleet worker process; ``status`` /
``config`` view and edit the per-agent schedule. The process layer is reached
through module attributes (``process.start_detached`` / ``process.stop_monitor``
/ ``loop.run_monitor_loop``) — the established broker-style indirection.
"""

from datetime import UTC, datetime

import click

from cafleet import broker, output
from cafleet.cli._helpers import ensure_tmux_or_die, require_fleet_id
from cafleet.monitor import DEFAULT_TICK_SECONDS, loop, process


@click.group()
def monitor() -> None:
    """Supervision scheduler (heartbeat) commands."""


def _require_live_fleet(fleet_id: int) -> None:
    fleet = broker.get_fleet(fleet_id)
    if fleet is None or fleet["deleted_at"] is not None:
        raise click.ClickException(f"fleet {fleet_id} not found")


@monitor.command("start")
@click.option(
    "--tick",
    type=click.IntRange(min=1),
    default=DEFAULT_TICK_SECONDS,
    show_default=True,
    help="Scan-tick cadence in seconds.",
)
@click.option(
    "--foreground",
    "foreground",
    is_flag=True,
    default=False,
    help="Run the loop in the current pane instead of detaching (debugging).",
)
@click.pass_context
def monitor_start(ctx: click.Context, tick: int, foreground: bool) -> None:
    """Start the fleet's monitor (detached by default)."""
    require_fleet_id(ctx)
    fleet_id = ctx.obj["fleet_id"]
    _require_live_fleet(fleet_id)
    ensure_tmux_or_die()

    if foreground:
        loop.run_monitor_loop(fleet_id, tick)
        return

    result = process.start_detached(fleet_id, tick)
    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "running": result.ok,
                    "pid": result.pid,
                    "tick_seconds": result.tick_seconds,
                }
            )
        )
    else:
        click.echo(result.message)
    if not result.ok:
        ctx.exit(1)


@monitor.command("stop")
@click.pass_context
def monitor_stop(ctx: click.Context) -> None:
    """Stop the fleet's monitor (idempotent)."""
    require_fleet_id(ctx)
    fleet_id = ctx.obj["fleet_id"]
    result = process.stop_monitor(fleet_id)
    if ctx.obj["json_output"]:
        click.echo(output.format_json({"stopped": result.stopped, "pid": result.pid}))
    else:
        click.echo(result.message)


@monitor.command("status")
@click.pass_context
def monitor_status(ctx: click.Context) -> None:
    """Show monitor liveness plus the per-agent schedule table."""
    require_fleet_id(ctx)
    fleet_id = ctx.obj["fleet_id"]
    _require_live_fleet(fleet_id)

    now = datetime.now(UTC)
    row = broker.read_monitor_runtime(fleet_id)
    if row is None:
        runtime = {
            "running": False,
            "pid": None,
            "tick_seconds": None,
            "last_tick_at": None,
            "last_tick_age_seconds": None,
            "started_at": None,
        }
    else:
        age = None
        if row["last_tick_at"] is not None:
            age = int(
                (now - datetime.fromisoformat(row["last_tick_at"])).total_seconds()
            )
        runtime = {
            "running": broker.monitor_is_live(fleet_id, now),
            "pid": row["pid"],
            "tick_seconds": row["tick_seconds"],
            "last_tick_at": row["last_tick_at"],
            "last_tick_age_seconds": age,
            "started_at": row["started_at"],
        }

    agents = [
        {
            "agent_id": t["agent_id"],
            "name": t["name"],
            "role": "director" if t["is_director"] else "member",
            "interval_seconds": t["interval_seconds"],
            "last_ping_at": t["last_ping_at"],
            "enabled": t["enabled"],
            "pending_count": t["pending_count"],
        }
        for t in broker.list_monitor_targets(fleet_id)
    ]
    payload = {"runtime": runtime, "agents": agents}

    if ctx.obj["json_output"]:
        click.echo(output.format_json(payload))
    else:
        click.echo(output.format_monitor_status(payload))


@monitor.command("config")
@click.option("--agent-id", "agent_id", type=int, required=True, help="Target agent.")
@click.option(
    "--interval",
    "interval",
    type=click.IntRange(min=1),
    default=None,
    help="New ping interval in seconds.",
)
@click.option(
    "--enable", "enable", is_flag=True, default=False, help="Enable monitoring."
)
@click.option(
    "--disable", "disable", is_flag=True, default=False, help="Disable monitoring."
)
@click.pass_context
def monitor_config(
    ctx: click.Context,
    agent_id: int,
    interval: int | None,
    enable: bool,
    disable: bool,
) -> None:
    """Show or edit an agent's monitor schedule."""
    require_fleet_id(ctx)
    fleet_id = ctx.obj["fleet_id"]
    if enable and disable:
        raise click.UsageError("--enable and --disable are mutually exclusive.")
    enabled = True if enable else (False if disable else None)

    if interval is None and enabled is None:
        cfg = broker.get_monitor_config(fleet_id, agent_id)
        if cfg is None:
            raise click.ClickException(
                f"agent {agent_id} is not enrolled in monitoring for fleet {fleet_id}."
            )
    else:
        cfg = broker.update_monitor_config(
            fleet_id, agent_id, interval_seconds=interval, enabled=enabled
        )

    if ctx.obj["json_output"]:
        click.echo(output.format_json(cfg))
    else:
        click.echo(output.format_monitor_config(cfg))
