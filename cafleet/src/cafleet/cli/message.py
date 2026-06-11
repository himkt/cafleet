"""``cafleet message`` — message broker commands."""

import click

from cafleet import broker, output
from cafleet.cli._helpers import (
    client_command,
    full_flag,
    full_flag_with_help,
    quiet_flag,
    quiet_flag_with_help,
)


@click.group()
def message() -> None:
    """Message broker commands."""


@message.command("send")
@click.option("--agent-id", type=int, required=True, help="Agent ID")
@click.option("--to", type=int, required=True, help="Recipient agent ID")
@click.option("--text", required=True, help="Message text")
@full_flag
@quiet_flag
@click.pass_context
@client_command(
    requires_agent_fleet=True,
    text_formatter=lambda r, *, full, quiet: (
        str(r["task"]["task_id"])
        if quiet
        else "Message sent.\n" + output.format_task(r, full=full)
    ),
    truncates_task_text=True,
)
def message_send(ctx, agent_id, to, text, full, quiet):
    """Send a unicast message to another agent."""
    fleet_id = ctx.obj["fleet_id"]
    return broker.send_message(
        fleet_id,
        agent_id,
        to,
        text,
    )


@message.command("broadcast")
@click.option("--agent-id", type=int, required=True, help="Agent ID")
@click.option("--text", required=True, help="Message text")
@full_flag
@click.pass_context
@client_command(
    text_formatter=lambda r, *, full: (
        output.format_task(r[0]["task"], full=True)
        if full
        else f"broadcast id={r[0]['task']['task_id']} "
        f"recipients={r[0]['notifications_sent_count']}"
    ),
    truncates_task_text=True,
)
def message_broadcast(ctx, agent_id, text, full):
    """Broadcast a message to all agents."""
    return broker.broadcast_message(
        ctx.obj["fleet_id"],
        agent_id,
        text,
    )


@message.command("poll")
@click.option("--agent-id", type=int, required=True, help="Agent ID")
@full_flag_with_help("Disable body truncation.")
@click.pass_context
@client_command(
    requires_agent_fleet=True,
    text_formatter=lambda r, *, full: output.format_indexed_list(
        r, lambda t: output.format_task(t, full=full), "No messages found."
    ),
    truncates_task_text=True,
)
def message_poll(ctx, agent_id, full):
    """Poll inbox for un-acked messages."""
    return broker.poll_tasks(agent_id)


@message.command("ack")
@click.option("--agent-id", type=int, required=True, help="Agent ID")
@click.option("--task-id", type=int, required=True, help="Task ID to acknowledge")
@full_flag_with_help("Disable body truncation.")
@quiet_flag_with_help("Print only the task id.")
@click.pass_context
@client_command(
    requires_agent_fleet=True,
    text_formatter=lambda r, *, full, quiet: (
        str(r["task"]["task_id"])
        if quiet
        else "Message acknowledged.\n" + output.format_task(r, full=full)
    ),
    truncates_task_text=True,
)
def message_ack(ctx, agent_id, task_id, full, quiet):
    """Acknowledge receipt of a message."""
    return broker.ack_task(agent_id, task_id)


@message.command("cancel")
@click.option("--agent-id", type=int, required=True, help="Agent ID")
@click.option("--task-id", type=int, required=True, help="Task ID to cancel")
@full_flag_with_help("Disable body truncation.")
@click.pass_context
@client_command(
    requires_agent_fleet=True,
    text_formatter=lambda r, *, full: (
        "Task canceled.\n" + output.format_task(r, full=full)
    ),
    truncates_task_text=True,
)
def message_cancel(ctx, agent_id, task_id, full):
    """Cancel (retract) a sent message."""
    return broker.cancel_task(agent_id, task_id)


@message.command("show")
@click.option("--agent-id", type=int, required=True, help="Agent ID")
@click.option("--task-id", type=int, required=True, help="Task ID to retrieve")
@full_flag_with_help("Disable body truncation.")
@click.pass_context
@client_command(
    requires_agent_fleet=True,
    text_formatter=lambda r, *, full: output.format_task(r, full=full),
    truncates_task_text=True,
)
def message_show(ctx, agent_id, task_id, full):
    """Get details of a specific task."""
    fleet_id = ctx.obj["fleet_id"]
    return broker.get_task(fleet_id, task_id)
