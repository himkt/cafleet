"""``cafleet message`` — message broker commands."""

import click

from cafleet import broker, output
from cafleet.cli._helpers import (
    client_command,
    ensure_skills_current,
    fleet_id_option,
    from_member_id_option,
    full_flag,
    json_flag,
    member_id_option,
    quiet_flag,
    text_body_options,
    to_member_id_option,
)
from cafleet.cli._text_input import read_text_input


@click.group()
def message() -> None:
    """Message broker commands."""
    ensure_skills_current()


@message.command("send")
@fleet_id_option
@from_member_id_option
@to_member_id_option
@text_body_options("Message body (inline).")
@full_flag
@quiet_flag
@json_flag
@click.pass_context
@client_command(
    requires_member_fleet=True,
    member_kwarg="from_member_id",
    text_formatter=lambda r, *, full, quiet: (
        str(r["task"]["task_id"])
        if quiet
        else "Message sent.\n" + output.format_task(r, full=full)
    ),
)
def message_send(
    ctx, from_member_id, to_member_id, text, text_file, full, quiet, json_output
):
    """Send a unicast message to another member."""
    fleet_id = ctx.obj["fleet_id"]
    body = read_text_input(text, text_file)
    return broker.send_message(
        fleet_id,
        from_member_id,
        to_member_id,
        body,
    )


@message.command("broadcast")
@fleet_id_option
@from_member_id_option
@text_body_options("Message body (inline).")
@full_flag
@json_flag
@click.pass_context
@client_command(
    text_formatter=lambda r, *, full: (
        output.format_task(r[0]["task"], full=True)
        if full
        else f"broadcast id={r[0]['task']['task_id']} "
        f"recipients={r[0]['recipients']} delivered={r[0]['delivered']}"
    ),
)
def message_broadcast(ctx, from_member_id, text, text_file, full, json_output):
    """Broadcast a message to all members."""
    body = read_text_input(text, text_file)
    return broker.broadcast_message(
        ctx.obj["fleet_id"],
        from_member_id,
        body,
    )


@message.command("poll")
@fleet_id_option
@member_id_option
@full_flag
@json_flag
@click.pass_context
@client_command(
    requires_member_fleet=True,
    text_formatter=lambda r, *, full: output.format_indexed_list(
        r, lambda t: output.format_task(t, full=full), "No messages found."
    ),
)
def message_poll(ctx, member_id, full, json_output):
    """Poll inbox for un-acked messages."""
    return broker.poll_tasks(member_id)


@message.command("ack")
@fleet_id_option
@member_id_option
@click.option("--task-id", type=int, required=True, help="Task ID to acknowledge")
@full_flag
@quiet_flag
@json_flag
@click.pass_context
@client_command(
    requires_member_fleet=True,
    text_formatter=lambda r, *, full, quiet: (
        str(r["task"]["task_id"])
        if quiet
        else "Message acknowledged.\n" + output.format_task(r, full=full)
    ),
)
def message_ack(ctx, member_id, task_id, full, quiet, json_output):
    """Acknowledge receipt of a message."""
    return broker.ack_task(member_id, task_id)


@message.command("cancel")
@fleet_id_option
@member_id_option
@click.option("--task-id", type=int, required=True, help="Task ID to cancel")
@full_flag
@json_flag
@click.pass_context
@client_command(
    requires_member_fleet=True,
    text_formatter=lambda r, *, full: (
        "Task canceled.\n" + output.format_task(r, full=full)
    ),
)
def message_cancel(ctx, member_id, task_id, full, json_output):
    """Cancel (retract) a sent message."""
    return broker.cancel_task(member_id, task_id)


@message.command("show")
@fleet_id_option
@member_id_option
@click.option("--task-id", type=int, required=True, help="Task ID to retrieve")
@full_flag
@json_flag
@click.pass_context
@client_command(
    requires_member_fleet=True,
    text_formatter=lambda r, *, full: output.format_task(r, full=full),
)
def message_show(ctx, member_id, task_id, full, json_output):
    """Get details of a specific task."""
    fleet_id = ctx.obj["fleet_id"]
    return broker.get_task(fleet_id, task_id)
