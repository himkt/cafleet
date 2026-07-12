"""Cross-submodule helpers and session context managers for the broker package."""

from contextlib import contextmanager
from datetime import UTC, datetime

from sqlalchemy import exists, func, select

from cafleet.db.engine import get_sync_sessionmaker
from cafleet.db.models import Member, Message

MONITORING_MEMBER_KIND = "monitoring-member"

MESSAGE_COLUMNS = tuple(Message.__table__.columns.keys())

NOT_BROADCAST_SUMMARY = Message.type != "broadcast_summary"

# Shared SQL expression extracting a member card's ``$.cafleet.kind`` (NULL → "")
# so a comparison against a non-empty kind constant selects identical rows.
CARD_KIND_SQL = func.coalesce(
    func.json_extract(Member.member_card_json, "$.cafleet.kind"), ""
)


@contextmanager
def read_session():
    sm = get_sync_sessionmaker()
    with sm() as session:
        yield session


@contextmanager
def write_session():
    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        yield session


def derive_member_kind(is_root_director: bool, card_kind: str | None) -> str:
    """Collapse a member to its 3-value ``kind``.

    ``card_kind`` values that do not match the monitoring-member constant
    (including the SQL-coalesced ``""``) fall through to ``member``.
    """
    if is_root_director:
        return "director"
    if card_kind == MONITORING_MEMBER_KIND:
        return "monitor"
    return "member"


def now_iso() -> str:
    return datetime.now(UTC).isoformat()


def placement_dict(row) -> dict:
    return {
        "backend": row.backend,
        "mux_session": row.mux_session,
        "mux_window_id": row.mux_window_id,
        "mux_pane_id": row.mux_pane_id,
        "coding_agent": row.coding_agent,
        "created_at": row.created_at,
    }


def member_is_active_in_fleet(session, member_id: int, fleet_id: int) -> bool:
    return session.execute(
        select(
            exists().where(
                Member.member_id == member_id,
                Member.fleet_id == fleet_id,
                Member.status == "active",
            )
        )
    ).scalar_one()


def row_to_message_dict(row) -> dict:
    return {col: getattr(row, col) for col in MESSAGE_COLUMNS}


def read_message(session, message_id: int) -> dict | None:
    row = session.execute(
        select(*(getattr(Message, col) for col in MESSAGE_COLUMNS)).where(
            Message.message_id == message_id
        )
    ).first()
    if row is None:
        return None
    return row_to_message_dict(row)


def list_messages_where(
    *filters,
    status: str | None = None,
) -> list[dict]:
    stmt = (
        select(*(getattr(Message, col) for col in MESSAGE_COLUMNS))
        .where(*filters, NOT_BROADCAST_SUMMARY)
        .order_by(Message.status_timestamp.desc())
    )
    if status is not None:
        stmt = stmt.where(Message.status_state == status)
    with read_session() as session:
        rows = session.execute(stmt).all()
    return [row_to_message_dict(row) for row in rows]
