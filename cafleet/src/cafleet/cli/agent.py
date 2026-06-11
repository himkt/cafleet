"""``cafleet agent`` — agent registry commands."""

import json

import click

from cafleet import broker, output
from cafleet.cli._helpers import client_command, full_flag


@click.group()
def agent() -> None:
    """Agent registry commands."""


@agent.command("register")
@click.option("--name", required=True, help="Agent name")
@click.option("--description", required=True, help="Agent description")
@click.option("--skills", default=None, help="Skills as JSON string")
@click.pass_context
@client_command(text_formatter=output.format_register)
def agent_register(ctx, name, description, skills):
    """Register a new agent with the broker."""
    parsed_skills = None
    if skills is not None:
        try:
            parsed_skills = json.loads(skills)
        except json.JSONDecodeError as exc:
            raise click.ClickException(f"Invalid JSON in --skills: {exc}") from exc
    return broker.register_agent(
        ctx.obj["fleet_id"],
        name,
        description,
        skills=parsed_skills,
    )


@agent.command("list")
@full_flag
@click.pass_context
@client_command(
    text_formatter=lambda agents, *, full: output.format_indexed_list(
        agents, lambda a: output.format_agent(a, full=full), "No agents found."
    ),
    renders_agent_card=True,
)
def agent_list(ctx, full):
    """List registered agents in the fleet."""
    return broker.list_agents(ctx.obj["fleet_id"])


@agent.command("show")
@click.option("--agent-id", type=int, required=True, help="Agent ID")
@click.option(
    "--id", "target_agent_id", type=int, required=True, help="Target agent ID"
)
@full_flag
@click.pass_context
@client_command(
    requires_agent_fleet=True,
    text_formatter=output.format_agent,
    renders_agent_card=True,
)
def agent_show(ctx, agent_id, target_agent_id, full):
    """Show detail for a specific agent."""
    fleet_id = ctx.obj["fleet_id"]
    result = broker.get_agent(target_agent_id, fleet_id)
    if result is None:
        raise click.ClickException(f"Agent {target_agent_id} not found")
    return result


@agent.command("deregister")
@click.option("--agent-id", type=int, required=True, help="Agent ID")
@click.pass_context
@client_command(
    requires_agent_fleet=True,
    text_formatter=lambda _: "Agent deregistered successfully.",
)
def agent_deregister(ctx, agent_id):
    """Deregister this agent from the broker."""
    deregistered = broker.deregister_agent(agent_id)
    if not deregistered:
        raise click.ClickException(
            f"agent {agent_id} not found or already deregistered."
        )
    return {"status": "deregistered"}
