"""``cafleet message`` — message broker commands."""

import click

from cafleet import broker, output
from cafleet.cli._helpers import (
    agent_id_option,
    client_command,
    ensure_skills_current,
    fleet_id_option,
    full_flag,
    quiet_flag,
    text_body_options,
)
from cafleet.cli._text_input import read_text_input


@click.group()
def message() -> None:
    """Message broker commands."""
    ensure_skills_current()


@message.command("send")
@fleet_id_option
@agent_id_option
@click.option("--to", type=int, required=True, help="Recipient agent ID")
@text_body_options("Message body (inline).")
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
)
def message_send(ctx, agent_id, to, text, text_file, full, quiet):
    """Send a unicast message to another agent."""
    fleet_id = ctx.obj["fleet_id"]
    body = read_text_input(text, text_file)
    return broker.send_message(
        fleet_id,
        agent_id,
        to,
        body,
    )


@message.command("broadcast")
@fleet_id_option
@agent_id_option
@text_body_options("Message body (inline).")
@full_flag
@click.pass_context
@client_command(
    text_formatter=lambda r, *, full: (
        output.format_task(r[0]["task"], full=True)
        if full
        else f"broadcast id={r[0]['task']['task_id']} "
        f"recipients={r[0]['recipients']} delivered={r[0]['delivered']}"
    ),
)
def message_broadcast(ctx, agent_id, text, text_file, full):
    """Broadcast a message to all agents."""
    body = read_text_input(text, text_file)
    return broker.broadcast_message(
        ctx.obj["fleet_id"],
        agent_id,
        body,
    )


@message.command("poll")
@fleet_id_option
@agent_id_option
@full_flag
@click.pass_context
@client_command(
    requires_agent_fleet=True,
    text_formatter=lambda r, *, full: output.format_indexed_list(
        r, lambda t: output.format_task(t, full=full), "No messages found."
    ),
)
def message_poll(ctx, agent_id, full):
    """Poll inbox for un-acked messages."""
    return broker.poll_tasks(agent_id)


@message.command("ack")
@fleet_id_option
@agent_id_option
@click.option("--task-id", type=int, required=True, help="Task ID to acknowledge")
@full_flag
@quiet_flag
@click.pass_context
@client_command(
    requires_agent_fleet=True,
    text_formatter=lambda r, *, full, quiet: (
        str(r["task"]["task_id"])
        if quiet
        else "Message acknowledged.\n" + output.format_task(r, full=full)
    ),
)
def message_ack(ctx, agent_id, task_id, full, quiet):
    """Acknowledge receipt of a message."""
    return broker.ack_task(agent_id, task_id)


@message.command("cancel")
@fleet_id_option
@agent_id_option
@click.option("--task-id", type=int, required=True, help="Task ID to cancel")
@full_flag
@click.pass_context
@client_command(
    requires_agent_fleet=True,
    text_formatter=lambda r, *, full: (
        "Task canceled.\n" + output.format_task(r, full=full)
    ),
)
def message_cancel(ctx, agent_id, task_id, full):
    """Cancel (retract) a sent message."""
    return broker.cancel_task(agent_id, task_id)


@message.command("show")
@fleet_id_option
@agent_id_option
@click.option("--task-id", type=int, required=True, help="Task ID to retrieve")
@full_flag
@click.pass_context
@client_command(
    requires_agent_fleet=True,
    text_formatter=lambda r, *, full: output.format_task(r, full=full),
)
def message_show(ctx, agent_id, task_id, full):
    """Get details of a specific task."""
    fleet_id = ctx.obj["fleet_id"]
    return broker.get_task(fleet_id, task_id)
