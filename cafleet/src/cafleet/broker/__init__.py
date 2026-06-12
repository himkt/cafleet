"""Sync SQLAlchemy data-access layer shared by the CLI and WebUI."""

from cafleet.broker._shared import ADMINISTRATOR_KIND
from cafleet.broker.agents import (
    deregister_agent,
    get_agent,
    get_agent_names,
    list_agents,
    list_fleet_agents,
    register_agent,
    update_placement_pane_id,
    verify_agent_fleet,
)
from cafleet.broker.fleets import create_fleet, delete_fleet, get_fleet, list_fleets
from cafleet.broker.members import list_members, list_members_with_activity
from cafleet.broker.messaging import (
    ack_task,
    broadcast_message,
    cancel_task,
    poll_tasks,
    send_message,
)
from cafleet.broker.queries import get_task, list_inbox, list_sent, list_timeline

__all__ = [
    "ADMINISTRATOR_KIND",
    "create_fleet",
    "list_fleets",
    "get_fleet",
    "delete_fleet",
    "register_agent",
    "get_agent",
    "list_agents",
    "deregister_agent",
    "update_placement_pane_id",
    "list_members",
    "list_members_with_activity",
    "verify_agent_fleet",
    "list_fleet_agents",
    "get_agent_names",
    "send_message",
    "broadcast_message",
    "poll_tasks",
    "ack_task",
    "cancel_task",
    "list_inbox",
    "list_sent",
    "list_timeline",
    "get_task",
]
