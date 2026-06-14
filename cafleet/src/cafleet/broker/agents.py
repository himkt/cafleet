"""Agent registry and placement."""

import json

import click
from sqlalchemy import and_, delete, exists, func, or_, select, update

from cafleet.broker import _shared, monitor
from cafleet.broker.fleets import get_fleet
from cafleet.db.models import Agent, AgentPlacement, Fleet, Task


def register_agent(
    fleet_id: int,
    name: str,
    description: str,
    skills: list[dict] | None = None,
    placement: dict | None = None,
    kind: str | None = None,
) -> dict:
    """Register a new agent in the fleet and optionally create its placement.

    Rejects soft-deleted fleets with a message that differs from the
    "not found" case so callers can surface the right recovery hint. When
    ``placement`` is supplied, the named Director must be active in the
    same fleet, must not be the Administrator, and must be the fleet's root
    Director (``fleets.director_agent_id``). A fleet has exactly one Director:
    nested teams are forbidden, so every member placement's
    ``director_agent_id`` equals the fleet root and a non-root value is
    rejected.

    Args:
        fleet_id: Fleet id the new agent belongs to.
        name: Short human-identifiable label.
        description: One-sentence purpose statement.
        skills: Optional list of skill dicts persisted into the agent's
            ``agent_card_json`` blob.
        placement: Optional dict carrying ``director_agent_id``,
            ``tmux_session``, ``tmux_window_id``, ``tmux_pane_id``, and
            ``coding_agent``. When present, an ``AgentPlacement`` row is
            created alongside the agent.
        kind: Optional ``agent_card_json.cafleet.kind`` marker. When set to
            ``_shared.MONITORING_MEMBER_KIND`` the new agent is the fleet's
            dedicated monitoring member: it is enrolled in ``monitor_config``
            and a second active one per fleet is rejected. Ordinary members
            pass ``None`` and are not enrolled.

    Returns:
        Dict with ``agent_id``, ``name``, and ``registered_at``.

    Raises:
        click.UsageError: If the fleet does not exist, is soft-deleted, the
            named Director is not active in the same fleet, or the placement
            ``director_agent_id`` is not the fleet's root Director.
        click.ClickException: If the named Director is the built-in
            Administrator, or the fleet already has an active monitoring member.
    """
    sess = get_fleet(fleet_id)
    if sess is None:
        raise click.UsageError(f"Fleet '{fleet_id}' not found.")
    if sess["deleted_at"] is not None:
        raise click.UsageError(f"fleet {fleet_id} is deleted")

    registered_at = _shared.now_iso()
    agent_card: dict[str, object] = {
        "name": name,
        "description": description,
        "skills": skills or [],
    }
    if kind is not None:
        agent_card["cafleet"] = {"kind": kind}

    with _shared.write_session() as session:
        if kind == _shared.MONITORING_MEMBER_KIND:
            # One monitoring member per fleet — the single enforcement site
            # (the CLI passes ``kind`` straight through without re-checking).
            existing = session.execute(
                select(Agent.agent_id).where(
                    Agent.fleet_id == fleet_id,
                    Agent.status == "active",
                    func.json_extract(Agent.agent_card_json, "$.cafleet.kind")
                    == _shared.MONITORING_MEMBER_KIND,
                )
            ).first()
            if existing is not None:
                raise click.ClickException(
                    f"fleet {fleet_id} already has an active monitoring member "
                    f"(agent {existing.agent_id}); only one is allowed."
                )

        if placement is not None:
            director_id = placement["director_agent_id"]
            director_card = session.execute(
                select(Agent.agent_card_json).where(
                    Agent.agent_id == director_id,
                    Agent.fleet_id == fleet_id,
                    Agent.status == "active",
                )
            ).scalar_one_or_none()
            if director_card is None:
                raise click.UsageError(
                    f"Director agent '{director_id}' not found or not active "
                    f"in fleet '{fleet_id}'."
                )
            if _shared.is_administrator(director_card):
                raise click.ClickException("Administrator cannot be a director")
            root_director_id = sess["director_agent_id"]
            if director_id != root_director_id:
                raise click.UsageError(
                    f"nested teams are not supported; placement director_agent_id "
                    f"{director_id} must equal the fleet root "
                    f"Director {root_director_id}."
                )

        agent = Agent(
            fleet_id=fleet_id,
            name=name,
            description=description,
            status="active",
            registered_at=registered_at,
            agent_card_json=json.dumps(agent_card),
        )
        session.add(agent)
        session.flush()
        agent_id = agent.agent_id
        if placement is not None:
            session.add(
                AgentPlacement(
                    agent_id=agent_id,
                    director_agent_id=placement["director_agent_id"],
                    tmux_session=placement["tmux_session"],
                    tmux_window_id=placement["tmux_window_id"],
                    tmux_pane_id=placement["tmux_pane_id"],
                    coding_agent=placement["coding_agent"],
                    created_at=registered_at,
                )
            )
            # Enroll ONLY the dedicated monitoring member in the heartbeat,
            # atomically with its placement insert. Ordinary members are no
            # longer auto-enrolled — the loop pings only the Director (enrolled
            # at fleet create) and the monitoring member.
            if kind == _shared.MONITORING_MEMBER_KIND:
                monitor.enroll_agent(session, agent_id)

    return {
        "agent_id": agent_id,
        "name": name,
        "registered_at": registered_at,
    }


def get_agent(agent_id: int, fleet_id: int) -> dict | None:
    """Return the active agent's detail (with placement) or None.

    Args:
        agent_id: Agent id to look up.
        fleet_id: Fleet id the agent must belong to.

    Returns:
        Dict with ``agent_id``, ``name``, ``description``, ``status``,
        ``registered_at``, ``kind`` (``"user"`` or the Administrator kind),
        and ``placement`` (the placement sub-dict or ``None``). Returns
        ``None`` if no active agent matches.
    """
    with _shared.read_session() as session:
        agent = session.execute(
            select(Agent).where(
                Agent.agent_id == agent_id,
                Agent.fleet_id == fleet_id,
                Agent.status == "active",
            )
        ).scalar_one_or_none()

        if agent is None:
            return None

        placement_row = session.execute(
            select(AgentPlacement).where(AgentPlacement.agent_id == agent_id)
        ).scalar_one_or_none()

    result: dict = {
        "agent_id": agent.agent_id,
        "name": agent.name,
        "description": agent.description,
        "status": agent.status,
        "registered_at": agent.registered_at,
        "kind": (
            _shared.ADMINISTRATOR_KIND
            if _shared.is_administrator(agent.agent_card_json)
            else "user"
        ),
        "placement": None,
    }
    if placement_row is not None:
        result["placement"] = _shared.placement_dict(placement_row)
    return result


def list_agents(fleet_id: int) -> list[dict]:
    """Return all active agents in the fleet."""
    stmt = select(
        Agent.agent_id,
        Agent.name,
        Agent.description,
        Agent.registered_at,
    ).where(
        Agent.fleet_id == fleet_id,
        Agent.status == "active",
    )
    with _shared.read_session() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "agent_id": row.agent_id,
            "name": row.name,
            "description": row.description,
            "status": "active",
            "registered_at": row.registered_at,
        }
        for row in rows
    ]


def deregister_agent(agent_id: int) -> bool:
    """Soft-delete the agent and drop its placement.

    Args:
        agent_id: Agent id to deregister.

    Returns:
        ``True`` if a row was flipped from ``active`` to ``deregistered``;
        ``False`` if no matching active agent existed.

    Raises:
        click.UsageError: If ``agent_id`` is the root Director of any
            fleet — torn down via ``cafleet fleet delete`` instead.
        click.ClickException: If ``agent_id`` is the built-in Administrator.
    """
    with _shared.write_session() as session:
        is_root_director = session.execute(
            select(exists().where(Fleet.director_agent_id == agent_id))
        ).scalar_one()
        if is_root_director:
            raise click.UsageError(
                "cannot deregister the root Director; "
                "use 'cafleet fleet delete' instead"
            )

        card_json = session.execute(
            select(Agent.agent_card_json).where(Agent.agent_id == agent_id)
        ).scalar_one_or_none()
        if card_json is not None and _shared.is_administrator(card_json):
            raise click.ClickException("Administrator cannot be deregistered")
        deregistered = session.execute(
            update(Agent)
            .where(
                Agent.agent_id == agent_id,
                Agent.status == "active",
            )
            .values(
                status="deregistered",
                deregistered_at=_shared.now_iso(),
            )
            .returning(Agent.agent_id)
        ).all()
        if deregistered:
            session.execute(
                delete(AgentPlacement).where(AgentPlacement.agent_id == agent_id)
            )
            # Runtime config has no historical value; drop it on the same
            # lifecycle as the placement.
            monitor.delete_agent_monitor_row(session, agent_id)
    return bool(deregistered)


def update_placement_pane_id(agent_id: int, pane_id: str) -> dict | None:
    """Patch the agent's placement with a freshly resolved tmux pane id.

    Called after ``split_window`` returns the spawned pane's id so the
    placement row reflects the live pane rather than the placeholder used
    during the initial INSERT.

    Args:
        agent_id: Agent id whose placement should be updated.
        pane_id: New ``tmux_pane_id`` value.

    Returns:
        The refreshed placement dict, or ``None`` if no placement row was
        affected.
    """
    with _shared.write_session() as session:
        updated = session.execute(
            update(AgentPlacement)
            .where(AgentPlacement.agent_id == agent_id)
            .values(tmux_pane_id=pane_id)
            .returning(AgentPlacement.agent_id)
        ).first()
        if updated is None:
            return None
        row = session.execute(
            select(AgentPlacement).where(AgentPlacement.agent_id == agent_id)
        ).scalar_one()
    return _shared.placement_dict(row)


def verify_agent_fleet(agent_id: int, fleet_id: int) -> bool:
    """Return True iff the agent belongs to the fleet (any status).

    Args:
        agent_id: Agent id to verify.
        fleet_id: Fleet id to check membership against.

    Returns:
        ``True`` if a matching row exists; ``False`` otherwise. Status is
        ignored — deregistered agents still pass.
    """
    with _shared.read_session() as session:
        return session.execute(
            select(
                exists().where(
                    Agent.agent_id == agent_id,
                    Agent.fleet_id == fleet_id,
                )
            )
        ).scalar_one()


def list_fleet_agents(fleet_id: int) -> list[dict]:
    """Return active agents plus deregistered agents that still own tasks.

    ``kind`` is derived in SQL via ``json_extract`` so the card blob never
    leaves SQLite — otherwise we would materialize every row's JSON just to
    compute a one-token discriminator. ``coalesce`` handles cards without a
    ``cafleet.kind`` path by substituting an empty string.
    """
    has_tasks = exists().where(
        or_(
            Task.context_id == Agent.agent_id,
            Task.from_agent_id == Agent.agent_id,
        )
    )
    kind_expr = func.coalesce(
        func.json_extract(Agent.agent_card_json, "$.cafleet.kind"), ""
    )
    stmt = select(
        Agent.agent_id,
        Agent.name,
        Agent.description,
        Agent.status,
        Agent.registered_at,
        kind_expr.label("kind_raw"),
    ).where(
        Agent.fleet_id == fleet_id,
        or_(
            Agent.status == "active",
            and_(Agent.status == "deregistered", has_tasks),
        ),
    )
    with _shared.read_session() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "agent_id": row.agent_id,
            "name": row.name,
            "description": row.description,
            "status": row.status,
            "registered_at": row.registered_at,
            "kind": (
                _shared.ADMINISTRATOR_KIND
                if row.kind_raw == _shared.ADMINISTRATOR_KIND
                else "user"
            ),
        }
        for row in rows
    ]


def get_agent_names(agent_ids: list[int]) -> dict[int, str]:
    """Batch ``agent_id → name`` lookup including deregistered agents."""
    if not agent_ids:
        return {}
    with _shared.read_session() as session:
        rows = session.execute(
            select(Agent.agent_id, Agent.name).where(Agent.agent_id.in_(agent_ids))
        ).all()
    return {row.agent_id: row.name for row in rows}
