"""``cafleet monitor`` — supervision-scheduler commands.

``start`` runs the `scan → ping → sleep` loop in-process — a coding agent
launches it as a background task and owns its lifetime (there is no detached
process and no ``stop`` command; stop the background task to stop it).
``status`` / ``config`` view and edit the per-member schedule. The loop is
reached through a module attribute (``loop.run_monitor_loop``) — the
established broker-style indirection.
"""

import re
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
from cafleet.monitor import DEFAULT_TICK_SECONDS, loop

_STALL_CLASSIFICATIONS = (
    "awaiting_user",
    "unknown",
    "finished",
    "working",
    "stall_candidate",
)
_SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")


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


@monitor.command("status")
@fleet_id_option
@json_flag
@click.pass_context
def monitor_status(ctx: click.Context, json_output: bool) -> None:
    """Show monitor liveness plus the per-member schedule table."""
    fleet_id = ctx.obj["fleet_id"]
    _require_live_fleet(fleet_id)

    now = datetime.now(UTC)
    # matches GET /api/monitor for WebUI/CLI parity (one `now` for runtime + members).
    payload = {
        "runtime": broker.monitor_runtime_payload(fleet_id, now),
        "members": broker.monitor_members_payload(fleet_id, now),
    }

    if json_output:
        click.echo(output.format_json(payload))
    else:
        click.echo(output.format_monitor_status(payload))


@monitor.command("config")
@fleet_id_option
@member_id_option
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
@json_flag
@click.pass_context
def monitor_config(
    ctx: click.Context,
    member_id: int,
    interval: int | None,
    enable: bool,
    disable: bool,
    json_output: bool,
) -> None:
    """Show or edit a member's monitor schedule."""
    fleet_id = ctx.obj["fleet_id"]
    if enable and disable:
        raise click.UsageError("--enable and --disable are mutually exclusive.")
    enabled = True if enable else (False if disable else None)

    if interval is None and enabled is None:
        cfg = broker.get_monitor_config(fleet_id, member_id)
        if cfg is None:
            raise click.ClickException(
                f"member {member_id} is not enrolled in monitoring "
                f"for fleet {fleet_id}."
            )
    else:
        cfg = broker.update_monitor_config(
            fleet_id, member_id, interval_seconds=interval, enabled=enabled
        )

    if json_output:
        click.echo(output.format_json(cfg))
    else:
        click.echo(output.format_monitor_config(cfg))


@monitor.group("stall")
def monitor_stall() -> None:
    """Inspect and update durable monitor stall episodes."""


def _validate_capture_identity(
    classification: str,
    captured_at: str | None,
    capture_sha256: str | None,
) -> None:
    if (captured_at is None) != (capture_sha256 is None):
        raise click.UsageError(
            "--captured-at and --capture-sha256 must be provided together."
        )
    if classification == "unknown":
        if captured_at is not None:
            raise click.UsageError(
                "--classification unknown must omit capture identity."
            )
        return
    if captured_at is None:
        raise click.UsageError(
            "readable classifications require --captured-at and --capture-sha256."
        )
    if not _SHA256_RE.fullmatch(capture_sha256 or ""):
        raise click.UsageError(
            "--capture-sha256 must be 64 lowercase hexadecimal characters."
        )
    try:
        parsed = datetime.fromisoformat(captured_at)
    except ValueError as exc:
        raise click.UsageError(
            "--captured-at must be a timezone-aware UTC ISO-8601 timestamp."
        ) from exc
    if parsed.tzinfo is None or parsed.utcoffset() != datetime.now(UTC).utcoffset():
        raise click.UsageError(
            "--captured-at must be a timezone-aware UTC ISO-8601 timestamp."
        )


@monitor_stall.command("observe")
@fleet_id_option
@member_id_option
@click.option(
    "--classification",
    type=click.Choice(_STALL_CLASSIFICATIONS),
    required=True,
)
@click.option("--captured-at", default=None)
@click.option("--capture-sha256", default=None)
@click.option("--stall-check", is_flag=True, default=False)
@click.option("--director-gate", is_flag=True, default=False)
@json_flag
@click.pass_context
def monitor_stall_observe(
    ctx: click.Context,
    member_id: int,
    classification: str,
    captured_at: str | None,
    capture_sha256: str | None,
    stall_check: bool,
    director_gate: bool,
    json_output: bool,
) -> None:
    """Record one typed pane observation and resolve its durable action."""
    if stall_check and director_gate:
        raise click.UsageError(
            "--stall-check and --director-gate are mutually exclusive."
        )
    _validate_capture_identity(classification, captured_at, capture_sha256)
    result = broker.observe_stall_episode(
        ctx.obj["fleet_id"],
        member_id,
        classification=classification,
        captured_at=captured_at,
        content_sha256=capture_sha256,
        stall_check=stall_check,
        director_gate=director_gate,
    )
    if json_output:
        click.echo(output.format_json(result))
        return
    reason = result["escalation_reason"] or "-"
    token = result["director_gate_token"] or "-"
    click.echo(
        f"member {result['member_id']}: {result['classification']}, "
        f"action {result['action']}, episode {result['episode_state']}, "
        f"reason {reason}, director gate {token}"
    )


@monitor_stall.command("ping-result")
@fleet_id_option
@member_id_option
@click.option("--success", is_flag=True, default=False)
@click.option("--failure", is_flag=True, default=False)
@json_flag
@click.pass_context
def monitor_stall_ping_result(
    ctx: click.Context,
    member_id: int,
    success: bool,
    failure: bool,
    json_output: bool,
) -> None:
    """Record the known result of one claimed fixed poll ping."""
    if success == failure:
        raise click.UsageError("exactly one of --success or --failure is required.")
    result = broker.record_stall_ping_result(
        ctx.obj["fleet_id"],
        member_id,
        success=success,
    )
    if json_output:
        click.echo(output.format_json(result))
        return
    reason = result["escalation_reason"] or "-"
    click.echo(
        f"member {result['member_id']}: episode {result['episode_state']}, "
        f"reason {reason}"
    )


@monitor_stall.command("pending")
@fleet_id_option
@json_flag
@click.pass_context
def monitor_stall_pending(ctx: click.Context, json_output: bool) -> None:
    """List durable escalation-pending ordinary members."""
    members = broker.list_pending_stall_escalations(ctx.obj["fleet_id"])
    if json_output:
        click.echo(output.format_json({"members": members}))
        return
    if not members:
        click.echo("(no pending stall escalations)")
        return
    for member in members:
        click.echo(
            f"member {member['member_id']} ({member['name']}): "
            f"{member['escalation_reason']}"
        )


@monitor.command("report-batch")
@fleet_id_option
@click.option("--director-gate-token", required=True)
@click.option("--finished-member-id", type=int, multiple=True)
@json_flag
@click.pass_context
def monitor_report_batch(
    ctx: click.Context,
    director_gate_token: str,
    finished_member_id: tuple[int, ...],
    json_output: bool,
) -> None:
    """Reconcile or create one token-gated aggregate Director report."""
    if not _SHA256_RE.fullmatch(director_gate_token):
        raise click.UsageError(
            "--director-gate-token must be 64 lowercase hexadecimal characters."
        )
    result = broker.report_monitor_batch(
        ctx.obj["fleet_id"],
        director_gate_token=director_gate_token,
        finished_member_ids=list(finished_member_id),
    )
    if json_output:
        click.echo(output.format_json(result))
        return

    def _display_id(value: int | None) -> str:
        return str(value) if value is not None else "-"

    click.echo(
        "monitor report batch: "
        f"created {_display_id(result['created_message_id'])}, "
        f"open {_display_id(result['open_message_id'])}, "
        f"preview {_display_id(result['preview_message_id'])} "
        f"{result['preview_outcome']}, "
        f"{len(result['escalated_member_ids'])} escalated, "
        f"{len(result['finished_member_ids'])} finished"
    )
