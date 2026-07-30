"""Member registry, placement, roster, and activity proxies."""

import json
from datetime import UTC, datetime

import click
from sqlalchemy import and_, delete, exists, func, or_, select, update

from cafleet.broker import _shared, monitor
from cafleet.broker.fleets import get_fleet
from cafleet.db.models import Fleet, Member, MemberPlacement, Message


def register_member(
    fleet_id: int,
    name: str,
    description: str,
    skills: list[dict] | None = None,
    placement: dict | None = None,
    kind: str | None = None,
) -> dict:
    """Register a new member in the fleet and optionally create its placement.

    Rejects soft-deleted fleets with a message that differs from the
    "not found" case so callers can surface the right recovery hint. The
    ``placement`` dict carries no director id — the fleet row
    (``fleets.director_member_id``) is the single source of the Director
    identity, so nested teams are impossible by construction. When
    ``placement`` is supplied, the fleet's root Director must be an active
    member of the fleet (a loud invariant failure otherwise, since the value
    is not caller input).

    Args:
        fleet_id: Fleet id the new member belongs to.
        name: Short human-identifiable label.
        description: One-sentence purpose statement.
        skills: Optional list of skill dicts persisted into the member's
            ``member_card_json`` blob.
        placement: Optional dict carrying ``backend``, ``mux_session``,
            ``mux_window_id``, ``mux_pane_id``, and ``coding_agent``. When
            present, a ``MemberPlacement`` row is created alongside the
            member.
        kind: Optional ``member_card_json.cafleet.kind`` marker. When set to
            ``_shared.MONITORING_MEMBER_KIND`` the new member is the fleet's
            dedicated monitoring member — the unenrolled *watcher*: it is NOT
            enrolled in ``monitor_config`` (located by its kind marker instead),
            must be pane-bound, and a second active one per fleet is rejected.
            Every other pane-bound ordinary member (``kind`` left ``None``) is
            enrolled in ``monitor_config`` @720; the root Director is enrolled
            at ``create_fleet`` @180.

    Returns:
        Dict with ``member_id``, ``name``, and ``registered_at``.

    Raises:
        click.UsageError: If the fleet does not exist.
        click.ClickException: If the fleet is soft-deleted, the fleet's root
            Director is not an active member (the invariant guard), the fleet
            already has an active monitoring member, or a monitoring member is
            registered without a placement.
    """
    sess = get_fleet(fleet_id)
    if sess is None:
        raise click.UsageError(f"Fleet '{fleet_id}' not found.")
    if sess["deleted_at"] is not None:
        raise click.ClickException(f"fleet {fleet_id} is deleted")

    registered_at = _shared.now_iso()
    member_card: dict[str, object] = {
        "name": name,
        "description": description,
        "skills": skills or [],
    }
    if kind is not None:
        member_card["cafleet"] = {"kind": kind}

    with _shared.write_session() as session:
        if kind == _shared.MONITORING_MEMBER_KIND:
            # A monitoring member must be pane-bound: it runs the heartbeat loop
            # and receives the loop's wake triggers in its own pane. Without a
            # placement it would consume the one-per-fleet slot yet have no pane
            # to run the loop in or receive wakes.
            if placement is None:
                raise click.ClickException(
                    "a monitoring member must be pane-bound; register it via "
                    "'cafleet member create --role monitor' (placement required)."
                )
            # One monitoring member per fleet — the single enforcement site
            # (the CLI passes ``kind`` straight through without re-checking).
            existing = session.execute(
                select(Member.member_id).where(
                    Member.fleet_id == fleet_id,
                    Member.status == "active",
                    _shared.CARD_KIND_SQL == _shared.MONITORING_MEMBER_KIND,
                )
            ).first()
            if existing is not None:
                raise click.ClickException(
                    f"fleet {fleet_id} already has an active monitoring member "
                    f"(member {existing.member_id}); only one is allowed."
                )

        if placement is not None:
            # Invariant guard, not input validation: the Director id comes from
            # the fleet row itself, so a violation means a corrupted bootstrap.
            root_director_id = sess["director_member_id"]
            if root_director_id is None or not _shared.member_is_active_in_fleet(
                session, root_director_id, fleet_id
            ):
                raise click.ClickException(
                    f"fleet {fleet_id}'s root Director "
                    f"(member {root_director_id}) is not active."
                )

        member = Member(
            fleet_id=fleet_id,
            name=name,
            description=description,
            status="active",
            registered_at=registered_at,
            member_card_json=json.dumps(member_card),
        )
        session.add(member)
        session.flush()
        member_id = member.member_id
        if placement is not None:
            session.add(
                MemberPlacement(
                    member_id=member_id,
                    backend=placement["backend"],
                    mux_session=placement["mux_session"],
                    mux_window_id=placement["mux_window_id"],
                    mux_pane_id=placement["mux_pane_id"],
                    coding_agent=placement["coding_agent"],
                    created_at=registered_at,
                )
            )
            # Enroll every pane-bound ordinary member in the heartbeat @720,
            # atomically with its placement insert. The dedicated monitoring
            # member (the unenrolled watcher, located by kind) is excluded;
            # the root Director is enrolled separately at create_fleet.
            if kind != _shared.MONITORING_MEMBER_KIND:
                monitor.enroll_member(
                    session, member_id, interval=monitor.MEMBER_PING_INTERVAL_SECONDS
                )

    return {
        "member_id": member_id,
        "name": name,
        "registered_at": registered_at,
    }


def get_member(member_id: int, fleet_id: int) -> dict | None:
    """Return the active member's detail (with placement) or None.

    Args:
        member_id: Member id to look up.
        fleet_id: Fleet id the member must belong to.

    Returns:
        Dict with ``member_id``, ``name``, ``description``, ``status``,
        ``registered_at``, ``kind`` (``director`` / ``monitor`` /
        ``member``), ``skills`` (the card's skills list,
        usually ``[]``), and ``placement`` (the placement sub-dict or
        ``None``). Returns ``None`` if no active member matches.
    """
    with _shared.read_session() as session:
        member = session.execute(
            select(Member).where(
                Member.member_id == member_id,
                Member.fleet_id == fleet_id,
                Member.status == "active",
            )
        ).scalar_one_or_none()

        if member is None:
            return None

        placement_row = session.execute(
            select(MemberPlacement).where(MemberPlacement.member_id == member_id)
        ).scalar_one_or_none()

        is_root_director = session.execute(
            select(
                exists().where(
                    Fleet.fleet_id == fleet_id,
                    Fleet.director_member_id == member_id,
                )
            )
        ).scalar_one()

    card = json.loads(member.member_card_json) if member.member_card_json else {}
    result: dict = {
        "member_id": member.member_id,
        "name": member.name,
        "description": member.description,
        "status": member.status,
        "registered_at": member.registered_at,
        "kind": _shared.derive_member_kind(
            is_root_director, card.get("cafleet", {}).get("kind")
        ),
        "skills": card.get("skills", []),
        "placement": None,
    }
    if placement_row is not None:
        result["placement"] = _shared.placement_dict(placement_row)
    return result


def deregister_member(member_id: int) -> bool:
    """Soft-delete the member and drop its placement.

    Args:
        member_id: Member id to deregister.

    Returns:
        ``True`` if a row was flipped from ``active`` to ``deregistered``;
        ``False`` if no matching active member existed.

    Raises:
        click.ClickException: If ``member_id`` is the root Director of any
            fleet (torn down via ``cafleet fleet delete`` instead).
    """
    with _shared.write_session() as session:
        is_root_director = session.execute(
            select(exists().where(Fleet.director_member_id == member_id))
        ).scalar_one()
        if is_root_director:
            raise click.ClickException(
                "cannot deregister the root Director; "
                "use 'cafleet fleet delete' instead"
            )

        deregistered = session.execute(
            update(Member)
            .where(
                Member.member_id == member_id,
                Member.status == "active",
            )
            .values(
                status="deregistered",
                deregistered_at=_shared.now_iso(),
            )
            .returning(Member.member_id)
        ).all()
        if deregistered:
            session.execute(
                delete(MemberPlacement).where(MemberPlacement.member_id == member_id)
            )
            # Runtime config has no historical value; drop it on the same
            # lifecycle as the placement.
            monitor.delete_member_monitor_row(session, member_id)
    return bool(deregistered)


def update_placement_pane_id(member_id: int, pane_id: str) -> dict | None:
    """Patch the member's placement with a freshly resolved tmux pane id.

    Called after ``split_window`` returns the spawned pane's id so the
    placement row reflects the live pane rather than the placeholder used
    during the initial INSERT.

    Args:
        member_id: Member id whose placement should be updated.
        pane_id: New ``mux_pane_id`` value.

    Returns:
        The refreshed placement dict, or ``None`` if no placement row was
        affected.
    """
    with _shared.write_session() as session:
        updated = session.execute(
            update(MemberPlacement)
            .where(MemberPlacement.member_id == member_id)
            .values(mux_pane_id=pane_id)
            .returning(MemberPlacement.member_id)
        ).first()
        if updated is None:
            return None
        row = session.execute(
            select(MemberPlacement).where(MemberPlacement.member_id == member_id)
        ).scalar_one()
    return _shared.placement_dict(row)


def verify_member_fleet(member_id: int, fleet_id: int) -> bool:
    """Return True iff the member belongs to the fleet (any status).

    Args:
        member_id: Member id to verify.
        fleet_id: Fleet id to check membership against.

    Returns:
        ``True`` if a matching row exists; ``False`` otherwise. Status is
        ignored — deregistered members still pass.
    """
    with _shared.read_session() as session:
        return session.execute(
            select(
                exists().where(
                    Member.member_id == member_id,
                    Member.fleet_id == fleet_id,
                )
            )
        ).scalar_one()


def get_member_names(member_ids: list[int]) -> dict[int, str]:
    """Batch ``member_id → name`` lookup including deregistered members."""
    if not member_ids:
        return {}
    with _shared.read_session() as session:
        rows = session.execute(
            select(Member.member_id, Member.name).where(
                Member.member_id.in_(member_ids)
            )
        ).all()
    return {row.member_id: row.name for row in rows}


def list_members(fleet_id: int) -> list[dict]:
    """Return every active registry entry with kind, placement, and activity.

    The single CLI list shape: active rows LEFT OUTER JOINed to their
    placements (placementless rows included, the root Director and monitoring
    member too), joined against ``fleets`` for the ``is_root`` flag, the card
    ``kind`` derived in SQL via ``json_extract``, both collapsed by
    :func:`cafleet.broker._shared.derive_member_kind`.

    ``last_sent`` / ``last_recv`` / ``last_ack`` aggregate ``status_timestamp``
    over the ``messages`` table per member. All three filter ``Message.type !=
    'broadcast_summary'`` (mirrors ``poll_messages``); broadcast_summary rows
    land in the broadcaster's own inbox with ``status_state='completed'``
    and would otherwise pollute every proxy for the broadcaster.

    Args:
        fleet_id: Fleet id to scope the query to.

    Returns:
        List of dicts each carrying ``member_id``, ``name``, ``kind``
        (``director`` / ``monitor`` / ``member``), ``placement`` (``None``
        for placementless rows), ``last_sent``, ``last_recv``, ``last_ack``
        (ISO timestamps or ``None``), and ``idle`` — the integer-second delta
        between ``now`` and the most recent of ``last_sent`` / ``last_recv``,
        or ``None`` when both are ``None``.
    """
    last_sent_sq = (
        select(func.max(Message.status_timestamp))
        .where(
            Message.from_member_id == Member.member_id,
            _shared.NOT_BROADCAST_SUMMARY,
        )
        .correlate(Member)
        .scalar_subquery()
    )
    last_recv_sq = (
        select(func.max(Message.status_timestamp))
        .where(
            Message.owner_member_id == Member.member_id,
            _shared.NOT_BROADCAST_SUMMARY,
        )
        .correlate(Member)
        .scalar_subquery()
    )
    last_ack_sq = (
        select(func.max(Message.status_timestamp))
        .where(
            Message.owner_member_id == Member.member_id,
            Message.status_state == "completed",
            _shared.NOT_BROADCAST_SUMMARY,
        )
        .correlate(Member)
        .scalar_subquery()
    )
    stmt = (
        select(
            Member.member_id,
            Member.name,
            MemberPlacement.member_id.label("placement_member_id"),
            MemberPlacement.backend,
            MemberPlacement.mux_session,
            MemberPlacement.mux_window_id,
            MemberPlacement.mux_pane_id,
            MemberPlacement.coding_agent,
            MemberPlacement.created_at,
            Fleet.director_member_id.label("root_director_id"),
            _shared.CARD_KIND_SQL.label("card_kind"),
            last_sent_sq.label("last_sent"),
            last_recv_sq.label("last_recv"),
            last_ack_sq.label("last_ack"),
        )
        .join(Fleet, Fleet.fleet_id == Member.fleet_id)
        .outerjoin(MemberPlacement, Member.member_id == MemberPlacement.member_id)
        .where(
            Member.fleet_id == fleet_id,
            Member.status == "active",
        )
    )
    with _shared.read_session() as session:
        rows = session.execute(stmt).all()

    now = datetime.now(UTC)
    return [
        {
            "member_id": row.member_id,
            "name": row.name,
            "kind": _shared.derive_member_kind(
                row.member_id == row.root_director_id, row.card_kind
            ),
            "placement": (
                _shared.placement_dict(row)
                if row.placement_member_id is not None
                else None
            ),
            "last_sent": row.last_sent,
            "last_recv": row.last_recv,
            "last_ack": row.last_ack,
            "idle": _idle_seconds(now, row.last_sent, row.last_recv),
        }
        for row in rows
    ]


def list_roster(fleet_id: int, *, include_message_holders: bool = False) -> list[dict]:
    """Return the fleet's roster, placed or placementless, with the 3-value kind.

    Unlike :func:`list_members`, the roster covers the root Director, the
    monitoring member, ordinary members, and placementless
    rows — active rows LEFT OUTER JOINed to their placements. SQL supplies the
    ``fleets`` join (``is_root``) and the card ``kind`` via ``json_extract``;
    :func:`cafleet.broker._shared.derive_member_kind` is the single Python
    collapse. With ``include_message_holders=True`` (the WebUI roster),
    deregistered members that still own messages are also returned, so the
    audit-relevant deregistered set stays visible.

    Args:
        fleet_id: Fleet id to scope the query to.
        include_message_holders: When ``True``, also return deregistered members
            that still own messages (a message exists with ``owner_member_id``
            or ``from_member_id`` equal to the member id).

    Returns:
        List of dicts each carrying the :func:`list_members` row shape
        (``member_id``, ``name``, ``description``, ``status``,
        ``registered_at``, ``placement`` — ``None`` for placementless rows)
        plus ``kind`` (``director`` / ``monitor`` / ``member``).
    """
    status_filter = Member.status == "active"
    if include_message_holders:
        has_messages = exists().where(
            or_(
                Message.owner_member_id == Member.member_id,
                Message.from_member_id == Member.member_id,
            )
        )
        status_filter = or_(
            status_filter,
            and_(Member.status == "deregistered", has_messages),
        )
    stmt = (
        select(
            Member.member_id,
            Member.name,
            Member.description,
            Member.status,
            Member.registered_at,
            MemberPlacement.member_id.label("placement_member_id"),
            MemberPlacement.backend,
            MemberPlacement.mux_session,
            MemberPlacement.mux_window_id,
            MemberPlacement.mux_pane_id,
            MemberPlacement.coding_agent,
            MemberPlacement.created_at,
            Fleet.director_member_id.label("root_director_id"),
            _shared.CARD_KIND_SQL.label("card_kind"),
        )
        .join(Fleet, Fleet.fleet_id == Member.fleet_id)
        .outerjoin(MemberPlacement, Member.member_id == MemberPlacement.member_id)
        .where(
            Member.fleet_id == fleet_id,
            status_filter,
        )
    )
    with _shared.read_session() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "member_id": row.member_id,
            "name": row.name,
            "description": row.description,
            "status": row.status,
            "registered_at": row.registered_at,
            "placement": (
                _shared.placement_dict(row)
                if row.placement_member_id is not None
                else None
            ),
            "kind": _shared.derive_member_kind(
                row.member_id == row.root_director_id, row.card_kind
            ),
        }
        for row in rows
    ]


def _idle_seconds(
    now: datetime, last_sent: str | None, last_recv: str | None
) -> int | None:
    candidates = [t for t in (last_sent, last_recv) if t is not None]
    if not candidates:
        return None
    most_recent = datetime.fromisoformat(max(candidates))
    delta = (now - most_recent).total_seconds()
    return max(0, int(delta))
