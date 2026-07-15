"""Fleet CRUD."""

import json

import click
from sqlalchemy import and_, delete, exists, func, select, update

from cafleet.broker import _shared, monitor
from cafleet.db.models import Fleet, Member, MemberPlacement
from cafleet.multiplexer import MultiplexerContext

_DIRECTOR_NAME = "Director"
_DIRECTOR_DESCRIPTION = "Root Director for this fleet"


def create_fleet(
    name: str | None = None,
    *,
    director_context: MultiplexerContext,
    coding_agent: str,
    backend: str,
) -> dict:
    """Atomically bootstrap a fleet with its root Director.

    The fleet row is written first with ``director_member_id=NULL`` and
    back-filled once the Director's member row exists, so the column is
    DB-nullable even though the post-bootstrap invariant is NOT NULL.

    Args:
        name: Human-readable name for the fleet.
        director_context: Resolved multiplexer pane identity for the root
            Director, obtained via ``Multiplexer.context_discovery``.
        coding_agent: Operator-declared metadata that lands in the root
            Director's ``placement.coding_agent`` column. The CLI is the only
            caller and always supplies it.
        backend: The resolved multiplexer name (``mux.name``) recorded in the
            Director's ``placement.backend`` column.

    Returns:
        A dict carrying ``fleet_id``, ``name``, ``created_at``, and a
        ``director`` sub-dict with the Director's identity and placement
        metadata.
    """
    created_at = _shared.now_iso()
    director_card = {
        "name": _DIRECTOR_NAME,
        "description": _DIRECTOR_DESCRIPTION,
        "skills": [],
    }
    director_placement = {
        "backend": backend,
        "mux_session": director_context.session,
        "mux_window_id": director_context.window_id,
        "mux_pane_id": director_context.pane_id,
        "coding_agent": coding_agent,
        "created_at": created_at,
    }

    with _shared.write_session() as session:
        fleet = Fleet(
            name=name,
            created_at=created_at,
            deleted_at=None,
            director_member_id=None,
        )
        session.add(fleet)
        session.flush()
        fleet_id = fleet.fleet_id
        director = Member(
            fleet_id=fleet_id,
            name=_DIRECTOR_NAME,
            description=_DIRECTOR_DESCRIPTION,
            status="active",
            registered_at=created_at,
            deregistered_at=None,
            member_card_json=json.dumps(director_card),
        )
        session.add(director)
        session.flush()
        director_member_id = director.member_id
        session.add(MemberPlacement(member_id=director_member_id, **director_placement))
        session.flush()
        # Enroll the root Director in the heartbeat @180, atomically with fleet
        # creation. The Director is a watched member — the loop wakes the
        # monitoring member when the Director comes due on this interval.
        monitor.enroll_member(
            session,
            director_member_id,
            interval=monitor.DIRECTOR_PING_INTERVAL_SECONDS,
        )
        session.execute(
            update(Fleet)
            .where(Fleet.fleet_id == fleet_id)
            .values(director_member_id=director_member_id)
        )

    return {
        "fleet_id": fleet_id,
        "name": name,
        "created_at": created_at,
        "director": {
            "member_id": director_member_id,
            "name": _DIRECTOR_NAME,
            "description": _DIRECTOR_DESCRIPTION,
            "registered_at": created_at,
            "placement": director_placement,
        },
    }


def list_fleets() -> list[dict]:
    """Return non-soft-deleted fleets with their active member counts."""
    stmt = (
        select(
            Fleet.fleet_id,
            Fleet.director_member_id,
            Fleet.name,
            Fleet.created_at,
            func.count(Member.member_id).label("member_count"),
        )
        .select_from(Fleet)
        .outerjoin(
            Member,
            and_(
                Member.fleet_id == Fleet.fleet_id,
                Member.status == "active",
            ),
        )
        .where(Fleet.deleted_at.is_(None))
        .group_by(
            Fleet.fleet_id,
            Fleet.director_member_id,
            Fleet.name,
            Fleet.created_at,
        )
        .order_by(Fleet.created_at.desc(), Fleet.fleet_id.asc())
    )
    with _shared.read_session() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "fleet_id": row.fleet_id,
            "director_member_id": row.director_member_id,
            "name": row.name,
            "created_at": row.created_at,
            "member_count": row.member_count,
        }
        for row in rows
    ]


def get_fleet(fleet_id: int) -> dict | None:
    """Return the fleet row (including soft-deleted) or None.

    The returned dict exposes ``deleted_at`` so callers can distinguish a
    missing fleet from a soft-deleted one — ``register_member`` relies on
    this to reject soft-deleted fleets with a different error message.

    Args:
        fleet_id: Fleet id to look up.

    Returns:
        Dict with ``fleet_id``, ``name``, ``created_at``, ``deleted_at``,
        and ``director_member_id``, or ``None`` if no row exists.
    """
    with _shared.read_session() as session:
        result = session.execute(select(Fleet).where(Fleet.fleet_id == fleet_id))
        row = result.scalar_one_or_none()
    if row is None:
        return None
    return {
        "fleet_id": row.fleet_id,
        "name": row.name,
        "created_at": row.created_at,
        "deleted_at": row.deleted_at,
        "director_member_id": row.director_member_id,
    }


def delete_fleet(fleet_id: int) -> dict:
    """Soft-delete a fleet and deregister its members, in one transaction.

    Messages are left untouched so audit history survives. Idempotent: re-running
    against an already-deleted row short-circuits on the ``deleted_at IS NULL``
    guard and returns ``deregistered_count=0``.

    Args:
        fleet_id: Fleet id to soft-delete.

    Returns:
        Dict with ``deregistered_count`` — the number of members flipped from
        ``active`` to ``deregistered`` by this call.

    Raises:
        click.ClickException: If the fleet does not exist.
    """
    now = _shared.now_iso()
    with _shared.write_session() as session:
        fleet_exists = session.execute(
            select(exists().where(Fleet.fleet_id == fleet_id))
        ).scalar_one()
        if not fleet_exists:
            # ClickException exits 1 (matching ``fleet show``); UsageError
            # would print a Usage: banner + exit 2, wrong for a runtime miss.
            raise click.ClickException(f"fleet '{fleet_id}' not found.")

        soft_deleted = session.execute(
            update(Fleet)
            .where(
                Fleet.fleet_id == fleet_id,
                Fleet.deleted_at.is_(None),
            )
            .values(deleted_at=now)
            .returning(Fleet.fleet_id)
        ).all()
        if not soft_deleted:
            return {"deregistered_count": 0}

        deregistered = session.execute(
            update(Member)
            .where(
                Member.fleet_id == fleet_id,
                Member.status == "active",
            )
            .values(status="deregistered", deregistered_at=now)
            .returning(Member.member_id)
        ).all()
        deregistered_count = len(deregistered)
        members_in_fleet = select(Member.member_id).where(Member.fleet_id == fleet_id)
        session.execute(
            delete(MemberPlacement).where(
                MemberPlacement.member_id.in_(members_in_fleet)
            )
        )
        monitor.delete_fleet_monitor_rows(session, fleet_id)

    return {"deregistered_count": deregistered_count}
