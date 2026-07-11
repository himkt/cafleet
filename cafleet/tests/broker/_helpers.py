"""Shared broker-side test helpers (fleet + member setup)."""

from cafleet import broker
from cafleet.multiplexer import MultiplexerContext


def _create_fleet(name: str | None = None) -> dict:
    return broker.create_fleet(
        name=name,
        director_context=MultiplexerContext(
            session="main", window_id="@3", pane_id="%0"
        ),
        coding_agent="claude",
        backend="tmux",
    )


def _register_member(
    fleet_id: int,
    name: str = "test-member",
    description: str = "A test member",
    skills: list[dict] | None = None,
    placement: dict | None = None,
) -> dict:
    return broker.register_member(
        fleet_id=fleet_id,
        name=name,
        description=description,
        skills=skills,
        placement=placement,
    )


def _member_placement(pane_id: str | None, coding_agent: str = "claude") -> dict:
    return {
        "backend": "tmux",
        "mux_session": "main",
        "mux_window_id": "@3",
        "mux_pane_id": pane_id,
        "coding_agent": coding_agent,
    }


def _register_monitoring_member(
    fleet: dict, name: str = "watcher", pane_id: str = "%5"
) -> dict:
    """The dedicated monitoring member — the unenrolled watcher, located by kind."""
    return broker.register_member(
        fleet_id=fleet["fleet_id"],
        name=name,
        description="monitoring member",
        placement=_member_placement(pane_id),
        kind="monitoring-member",
    )


def _setup_two_members() -> tuple[int, int, int]:
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    a = _register_member(sid, name="sender")
    b = _register_member(sid, name="recipient")
    return sid, a["member_id"], b["member_id"]


def _setup_three_members() -> tuple[int, int, int, int]:
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    a = _register_member(sid, name="member-a")
    b = _register_member(sid, name="member-b")
    c = _register_member(sid, name="member-c")
    return sid, a["member_id"], b["member_id"], c["member_id"]
