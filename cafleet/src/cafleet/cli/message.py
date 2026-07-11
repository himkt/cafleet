"""``cafleet message`` — message broker commands."""

import click

from cafleet import broker, output
from cafleet.cli._helpers import (
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


def _require_member_in_fleet(member_id: int, fleet_id: int) -> None:
    """Fleet-gate the acting member; runs before the command body."""
    if not broker.verify_member_fleet(member_id, fleet_id):
        raise click.ClickException(f"member {member_id} is not in fleet {fleet_id}.")


@message.command("send")
@fleet_id_option
@from_member_id_option
@to_member_id_option
@text_body_options("Message body (inline).")
@full_flag
@quiet_flag
@json_flag
@click.pass_context
def message_send(
    ctx, from_member_id, to_member_id, text, text_file, full, quiet, json_output
):
    """Send a unicast message to another member."""
    fleet_id = ctx.obj["fleet_id"]
    try:
        _require_member_in_fleet(from_member_id, fleet_id)
        body = read_text_input(text, text_file)
        result = broker.send_message(
            fleet_id,
            from_member_id,
            to_member_id,
            body,
        )
        output.truncate_task_text(result, full=full)
        rendered = output.render_tasks_in_result(result, full=full)
        if json_output:
            click.echo(output.format_json(rendered))
        elif quiet:
            click.echo(str(result["task"]["task_id"]))
        else:
            click.echo("Message sent.\n" + output.format_task(result, full=full))
    except click.ClickException:
        raise
    except Exception as exc:
        raise click.ClickException(str(exc)) from exc


@message.command("broadcast")
@fleet_id_option
@from_member_id_option
@text_body_options("Message body (inline).")
@full_flag
@json_flag
@click.pass_context
def message_broadcast(ctx, from_member_id, text, text_file, full, json_output):
    """Broadcast a message to all members."""
    fleet_id = ctx.obj["fleet_id"]
    try:
        body = read_text_input(text, text_file)
        result = broker.broadcast_message(fleet_id, from_member_id, body)
        output.truncate_task_text(result, full=full)
        rendered = output.render_tasks_in_result(result, full=full)
        if json_output:
            click.echo(output.format_json(rendered))
        elif full:
            click.echo(output.format_task(result[0]["task"], full=True))
        else:
            click.echo(
                f"broadcast id={result[0]['task']['task_id']} "
                f"recipients={result[0]['recipients']} "
                f"delivered={result[0]['delivered']}"
            )
    except click.ClickException:
        raise
    except Exception as exc:
        raise click.ClickException(str(exc)) from exc


@message.command("poll")
@fleet_id_option
@member_id_option
@full_flag
@json_flag
@click.pass_context
def message_poll(ctx, member_id, full, json_output):
    """Poll inbox for un-acked messages."""
    fleet_id = ctx.obj["fleet_id"]
    try:
        _require_member_in_fleet(member_id, fleet_id)
        result = broker.poll_tasks(member_id)
        output.truncate_task_text(result, full=full)
        rendered = output.render_tasks_in_result(result, full=full)
        if json_output:
            click.echo(output.format_json(rendered))
        else:
            click.echo(
                output.format_indexed_list(
                    result,
                    lambda t: output.format_task(t, full=full),
                    "No messages found.",
                )
            )
    except click.ClickException:
        raise
    except Exception as exc:
        raise click.ClickException(str(exc)) from exc


@message.command("ack")
@fleet_id_option
@member_id_option
@click.option("--task-id", type=int, required=True, help="Task ID to acknowledge")
@full_flag
@quiet_flag
@json_flag
@click.pass_context
def message_ack(ctx, member_id, task_id, full, quiet, json_output):
    """Acknowledge receipt of a message."""
    fleet_id = ctx.obj["fleet_id"]
    try:
        _require_member_in_fleet(member_id, fleet_id)
        result = broker.ack_task(member_id, task_id)
        output.truncate_task_text(result, full=full)
        rendered = output.render_tasks_in_result(result, full=full)
        if json_output:
            click.echo(output.format_json(rendered))
        elif quiet:
            click.echo(str(result["task"]["task_id"]))
        else:
            click.echo(
                "Message acknowledged.\n" + output.format_task(result, full=full)
            )
    except click.ClickException:
        raise
    except Exception as exc:
        raise click.ClickException(str(exc)) from exc


@message.command("cancel")
@fleet_id_option
@member_id_option
@click.option("--task-id", type=int, required=True, help="Task ID to cancel")
@full_flag
@json_flag
@click.pass_context
def message_cancel(ctx, member_id, task_id, full, json_output):
    """Cancel (retract) a sent message."""
    fleet_id = ctx.obj["fleet_id"]
    try:
        _require_member_in_fleet(member_id, fleet_id)
        result = broker.cancel_task(member_id, task_id)
        output.truncate_task_text(result, full=full)
        rendered = output.render_tasks_in_result(result, full=full)
        if json_output:
            click.echo(output.format_json(rendered))
        else:
            click.echo("Task canceled.\n" + output.format_task(result, full=full))
    except click.ClickException:
        raise
    except Exception as exc:
        raise click.ClickException(str(exc)) from exc


@message.command("show")
@fleet_id_option
@member_id_option
@click.option("--task-id", type=int, required=True, help="Task ID to retrieve")
@full_flag
@json_flag
@click.pass_context
def message_show(ctx, member_id, task_id, full, json_output):
    """Get details of a specific task."""
    fleet_id = ctx.obj["fleet_id"]
    try:
        _require_member_in_fleet(member_id, fleet_id)
        result = broker.get_task(fleet_id, task_id)
        output.truncate_task_text(result, full=full)
        rendered = output.render_tasks_in_result(result, full=full)
        if json_output:
            click.echo(output.format_json(rendered))
        else:
            click.echo(output.format_task(result, full=full))
    except click.ClickException:
        raise
    except Exception as exc:
        raise click.ClickException(str(exc)) from exc
