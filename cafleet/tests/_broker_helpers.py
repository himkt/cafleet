"""Shared broker-side test helpers (session + agent setup)."""

from cafleet import broker
from cafleet.tmux import DirectorContext


def _create_session(label: str | None = None) -> dict:
    return broker.create_session(
        label=label,
        director_context=DirectorContext(session="main", window_id="@3", pane_id="%0"),
        coding_agent="claude",
    )


def _register_agent(
    session_id: str,
    name: str = "test-agent",
    description: str = "A test agent",
    skills: list[dict] | None = None,
    placement: dict | None = None,
) -> dict:
    return broker.register_agent(
        session_id=session_id,
        name=name,
        description=description,
        skills=skills,
        placement=placement,
    )


def _setup_two_agents() -> tuple[str, str, str]:
    session = _create_session()
    sid = session["session_id"]
    a = _register_agent(sid, name="sender")
    b = _register_agent(sid, name="recipient")
    return sid, a["agent_id"], b["agent_id"]


def _setup_three_agents() -> tuple[str, str, str, str]:
    session = _create_session()
    sid = session["session_id"]
    a = _register_agent(sid, name="agent-a")
    b = _register_agent(sid, name="agent-b")
    c = _register_agent(sid, name="agent-c")
    return sid, a["agent_id"], b["agent_id"], c["agent_id"]
