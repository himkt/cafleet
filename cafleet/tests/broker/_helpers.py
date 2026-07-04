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
    fleet_id: int,
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


def _member_placement(
    director_agent_id: int, pane_id: str | None, coding_agent: str = "claude"
) -> dict:
    return {
        "director_agent_id": director_agent_id,
        "tmux_session": "main",
        "tmux_window_id": "@3",
        "tmux_pane_id": pane_id,
        "coding_agent": coding_agent,
    }


def _register_monitoring_member(
    fleet: dict, name: str = "watcher", pane_id: str = "%5"
) -> dict:
    """The dedicated monitoring member — the unenrolled watcher, located by kind."""
    return broker.register_agent(
        fleet_id=fleet["fleet_id"],
        name=name,
        description="monitoring member",
        placement=_member_placement(fleet["director"]["agent_id"], pane_id),
        kind="monitoring-member",
    )


def _setup_two_agents() -> tuple[int, int, int]:
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    a = _register_agent(sid, name="sender")
    b = _register_agent(sid, name="recipient")
    return sid, a["agent_id"], b["agent_id"]


def _setup_three_agents() -> tuple[int, int, int, int]:
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    a = _register_agent(sid, name="agent-a")
    b = _register_agent(sid, name="agent-b")
    c = _register_agent(sid, name="agent-c")
    return sid, a["agent_id"], b["agent_id"], c["agent_id"]
