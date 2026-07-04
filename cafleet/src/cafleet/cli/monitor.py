"""``cafleet monitor`` — supervision-scheduler commands.

``start`` runs the `scan → ping → sleep` loop in-process — a coding agent
launches it as a background task and owns its lifetime (there is no detached
process and no ``stop`` command; stop the background task to stop it).
``status`` / ``config`` view and edit the per-agent schedule. The loop is
reached through a module attribute (``loop.run_monitor_loop``) — the
established broker-style indirection.
"""

from datetime import UTC, datetime

import click

from cafleet import broker, output
from cafleet.cli._helpers import (
    ensure_skills_current,
    ensure_tmux_or_die,
    fleet_id_option,
)
from cafleet.monitor import DEFAULT_TICK_SECONDS, loop


@click.group()
def monitor() -> None:
    """Supervision scheduler (heartbeat) commands."""
    ensure_skills_current()


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
    ensure_tmux_or_die()
    if broker.find_monitoring_member(fleet_id) is None:
        click.echo(
            f"Warning: fleet {fleet_id} has no monitoring member; the "
            f"monitor heartbeat will wake no agent. Spawn one first with "
            f"'cafleet member create --role monitor'.",
            err=True,
        )
    loop.run_monitor_loop(fleet_id, tick)


@monitor.command("status")
@fleet_id_option
@click.pass_context
def monitor_status(ctx: click.Context) -> None:
    """Show monitor liveness plus the per-agent schedule table."""
    fleet_id = ctx.obj["fleet_id"]
    _require_live_fleet(fleet_id)

    now = datetime.now(UTC)
    # matches GET /api/monitor for WebUI/CLI parity (one `now` for runtime + agents).
    runtime = broker.monitor_runtime_payload(fleet_id, now)

    agents = []
    for t in broker.list_monitor_targets(fleet_id):
        last_ping_at = t["last_ping_at"]
        age = (
            int((now - datetime.fromisoformat(last_ping_at)).total_seconds())
            if last_ping_at is not None
            else None
        )
        agents.append(
            {
                "agent_id": t["agent_id"],
                "name": t["name"],
                "role": "director" if t["is_director"] else "member",
                "interval_seconds": t["interval_seconds"],
                "last_ping_at": last_ping_at,
                "last_ping_age_seconds": age,
                "enabled": t["enabled"],
                "pending_count": t["pending_count"],
            }
        )
    payload = {"runtime": runtime, "agents": agents}

    if ctx.obj["json_output"]:
        click.echo(output.format_json(payload))
    else:
        click.echo(output.format_monitor_status(payload))


@monitor.command("config")
@fleet_id_option
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
