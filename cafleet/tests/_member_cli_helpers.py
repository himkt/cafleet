"""Shared constants + factories for member-CLI tests."""

DIRECTOR_ID = 11
MEMBER_ID = 22
OTHER_DIRECTOR_ID = 33
PANE_ID = "%7"
MEMBER_NAME = "Claude-B"


# Sentinel for ``_agent(placement=...)`` so callers can pass explicit None
# (meaning "no placement row, exercise the missing-placement branch") without
# being silently coerced back to a default valid placement.
_UNSET: object = object()


def _placement(
    *,
    director_agent_id: int = DIRECTOR_ID,
    tmux_pane_id: str | None = PANE_ID,
    coding_agent: str = "claude",
) -> dict:
    return {
        "director_agent_id": director_agent_id,
        "tmux_session": "main",
        "tmux_window_id": "@3",
        "tmux_pane_id": tmux_pane_id,
        "coding_agent": coding_agent,
        "created_at": "2026-04-16T08:00:00+00:00",
    }


def _agent(
    *,
    agent_id: int = MEMBER_ID,
    name: str = MEMBER_NAME,
    placement: dict | None | object = _UNSET,
) -> dict:
    resolved_placement = _placement() if placement is _UNSET else placement
    return {
        "agent_id": agent_id,
        "name": name,
        "description": "Test member",
        "status": "active",
        "registered_at": "2026-04-16T08:00:00+00:00",
        "kind": "user",
        "placement": resolved_placement,
    }
