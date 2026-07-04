"""``cafleet fleet`` — fleet management commands."""

import click

from cafleet import broker, output
from cafleet.cli._helpers import ensure_skills_current, fleet_id_option, full_flag
from cafleet.coding_agent import CODING_AGENTS
from cafleet.multiplexer import MULTIPLEXERS, TmuxError


@click.group()
def fleet() -> None:
    """Fleet management commands."""
    ensure_skills_current()


@fleet.command("create")
@click.option("--label", default=None, help="Optional human-readable label.")
@click.option(
    "--coding-agent",
    "coding_agent",
    type=click.Choice(list(CODING_AGENTS.keys())),
    default="claude",
    show_default=True,
    help="Coding-agent binary to spawn / declare for the placement.",
)
@click.option("--json", "as_json", is_flag=True, help="Output as JSON.")
@full_flag
@click.pass_context
def fleet_create(
    ctx: click.Context,
    label: str | None,
    coding_agent: str,
    as_json: bool,
    full: bool,
) -> None:
    """Create a new fleet (must be run inside a tmux session)."""
    try:
        MULTIPLEXERS["tmux"].ensure_available()
        director_ctx = MULTIPLEXERS["tmux"].context_discovery()
    except TmuxError as exc:
        raise click.ClickException(
            "cafleet fleet create must be run inside a tmux session"
        ) from exc

    result = broker.create_fleet(
        label=label,
        director_context=director_ctx,
        coding_agent=coding_agent,
    )

    if as_json or ctx.obj["json_output"]:
        click.echo(output.format_json(result))
    else:
        click.echo(output.format_fleet_create(result, full=full))


@fleet.command("list")
@click.option("--json", "as_json", is_flag=True, help="Output as JSON.")
@click.pass_context
def fleet_list(ctx: click.Context, as_json: bool) -> None:
    """List all fleets."""
    rows = broker.list_fleets()

    if as_json or ctx.obj["json_output"]:
        click.echo(output.format_json(rows))
    else:
        if not rows:
            click.echo("No fleets found.")
            return
        click.echo(
            f"{'FLEET_ID':<40} {'DIRECTOR':<40} {'LABEL':<20} "
            f"{'AGENTS':<8} {'CREATED_AT'}"
        )
        for r in rows:
            click.echo(
                f"{r['fleet_id']:<40} {r['director_agent_id'] or '':<40} "
                f"{r['label'] or '':<20} {r['agent_count']:<8} {r['created_at']}"
            )


@fleet.command("show")
@fleet_id_option
@click.option("--json", "as_json", is_flag=True, help="Output as JSON.")
@click.pass_context
def fleet_show(ctx: click.Context, as_json: bool) -> None:
    """Show details of a single fleet."""
    fleet_id = ctx.obj["fleet_id"]
    result = broker.get_fleet(fleet_id)
    if result is None:
        raise click.ClickException(f"fleet '{fleet_id}' not found.")

    if as_json or ctx.obj["json_output"]:
        click.echo(output.format_json(result))
    else:
        lines = [
            f"fleet_id: {result['fleet_id']}",
            f"label:      {result['label'] or ''}",
            f"created_at: {result['created_at']}",
        ]
        if result["deleted_at"] is not None:
            lines.append(f"deleted_at: {result['deleted_at']}")
        click.echo("\n".join(lines))


@fleet.command("delete")
@fleet_id_option
@click.pass_context
def fleet_delete(ctx: click.Context) -> None:
    """Soft-delete a fleet and deregister every active agent (idempotent)."""
    # No monitor-stop step: a running monitor loop self-terminates on its next
    # tick once the fleet is soft-deleted (monitor_tick → STOP), and
    # broker.delete_fleet removes the monitor_config + monitor_runtime rows.
    fleet_id = ctx.obj["fleet_id"]
    result = broker.delete_fleet(fleet_id)
    n = result["deregistered_count"]
    click.echo(f"Deleted fleet {fleet_id}. Deregistered {n} agents.")
