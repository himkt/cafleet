"""Shared broker-side test helpers (fleet + agent setup)."""

from cafleet import broker
from cafleet.multiplexer import MultiplexerContext


def _create_fleet(label: str | None = None) -> dict:
    return broker.create_fleet(
        label=label,
        director_context=MultiplexerContext(
            session="main", window_id="@3", pane_id="%0"
        ),
        coding_agent="claude",
    )


def _register_agent(
    fleet_id: str,
    name: str = "test-agent",
    description: str = "A test agent",
    skills: list[dict] | None = None,
    placement: dict | None = None,
) -> dict:
    return broker.register_agent(
        fleet_id=fleet_id,
        name=name,
        description=description,
        skills=skills,
        placement=placement,
    )


def _setup_two_agents() -> tuple[str, str, str]:
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    a = _register_agent(sid, name="sender")
    b = _register_agent(sid, name="recipient")
    return sid, a["agent_id"], b["agent_id"]


def _setup_three_agents() -> tuple[str, str, str, str]:
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    a = _register_agent(sid, name="agent-a")
    b = _register_agent(sid, name="agent-b")
    c = _register_agent(sid, name="agent-c")
    return sid, a["agent_id"], b["agent_id"], c["agent_id"]
