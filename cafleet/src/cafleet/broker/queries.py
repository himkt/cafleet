"""Read-only message queries (inbox/sent/timeline/get_message)."""

from sqlalchemy import exists, select

from cafleet.broker import _shared
from cafleet.db.models import Member, Message


def list_inbox(member_id: int) -> list[dict]:
    """Return raw message rows addressed to ``member_id`` (no broadcast_summary)."""
    return _shared.list_messages_where(Message.owner_member_id == member_id)


def list_sent(member_id: int) -> list[dict]:
    """Return raw message rows sent by ``member_id`` (no broadcast_summary)."""
    return _shared.list_messages_where(Message.from_member_id == member_id)


def list_timeline(fleet_id: int, limit: int = 200) -> list[dict]:
    """Return the fleet's recent messages in DESC ``status_timestamp`` order.

    ``broadcast_summary`` rows are filtered out so the timeline shows only
    delivery rows. Membership is tested via ``from_member_id`` joined to
    ``members.fleet_id``.

    Args:
        fleet_id: Fleet id to scope the query to.
        limit: Maximum number of rows to return (default 200).

    Returns:
        List of flat message dicts in DESC ``status_timestamp`` order.
    """
    stmt = (
        select(*(getattr(Message, col) for col in _shared.MESSAGE_COLUMNS))
        .join(Member, Message.from_member_id == Member.member_id)
        .where(
            Member.fleet_id == fleet_id,
            _shared.NOT_BROADCAST_SUMMARY,
        )
        .order_by(Message.status_timestamp.desc())
        .limit(limit)
    )
    with _shared.read_session() as session:
        rows = session.execute(stmt).all()
    return [_shared.row_to_message_dict(row) for row in rows]


def get_message(fleet_id: int, message_id: int) -> dict:
    """Return the message iff at least one of its endpoints lives in the fleet.

    Args:
        fleet_id: Fleet id used to gate visibility.
        message_id: Message id to fetch.

    Returns:
        Dict with ``message`` — the flat typed-column message dict.

    Raises:
        ValueError: If the message does not exist or neither endpoint belongs
            to ``fleet_id``.
    """
    with _shared.read_session() as session:
        message_dict = _shared.read_message(session, message_id)
        if message_dict is None:
            raise ValueError(f"Message {message_id} not found")

        endpoint_ids = [message_dict["from_member_id"]]
        to_id = message_dict["to_member_id"]
        if to_id is not None:
            endpoint_ids.append(to_id)
        in_fleet = session.execute(
            select(
                exists().where(
                    Member.member_id.in_(endpoint_ids),
                    Member.fleet_id == fleet_id,
                )
            )
        ).scalar_one()
        if not in_fleet:
            raise ValueError(f"Message {message_id} not found")

    return {"message": message_dict}
