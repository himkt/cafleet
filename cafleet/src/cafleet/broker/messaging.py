"""Message send/broadcast/poll/ack and inline-preview notification."""

from sqlalchemy import select, update
from sqlalchemy.dialects.sqlite import insert as sqlite_insert

from cafleet.broker import _shared
from cafleet.config import settings
from cafleet.db.models import Member, MemberPlacement, Message


def _try_notify_recipient(
    session, *, recipient_id: int, sender_id: int, message_dict: dict
) -> bool:
    """Best-effort inline-preview keystroke for the recipient's pane.

    Keystrokes a 2-line preview of the message itself into the recipient's
    pane — the recipient's TUI processes the keystrokes as a fresh user-turn
    input and the recipient acks via
    ``cafleet message ack --message-id <id>``. The queue remains the source
    of truth; failures are swallowed.
    """
    if recipient_id == sender_id:
        return False
    pane_id = session.execute(
        select(MemberPlacement.mux_pane_id).where(
            MemberPlacement.member_id == recipient_id
        )
    ).scalar_one_or_none()
    if pane_id is None:
        return False

    # Truncate before keystroking so a multi-KB body cannot dump itself into
    # the recipient's pane. Mirrors output.truncate_text's contract: same
    # limit (``settings.max_text_len`` / ``CAFLEET_MAX_TEXT_LEN``, default
    # 200) and same single-codepoint U+2026 suffix on overflow.
    preview_text = message_dict["text"]
    if len(preview_text) > settings.max_text_len:
        preview_text = preview_text[: settings.max_text_len] + "…"

    # Local import mirrors the broker's other multiplexer touchpoints and keeps
    # method-level monkeypatches live per call. Best-effort: an unresolvable
    # backend (broker not inside a pane) skips the preview rather than raising.
    from cafleet.multiplexer import MultiplexerError, resolve_multiplexer

    try:
        mux = resolve_multiplexer()
    except MultiplexerError:
        return False
    return mux.send_inline_preview(
        target_pane_id=pane_id,
        message_id=message_dict["message_id"],
        sender_id=sender_id,
        ts=message_dict["status_timestamp"],
        text=preview_text,
    )


def _insert_message(session, message_dict: dict) -> int:
    """INSERT a new message without a ``message_id`` and return the DB-assigned id."""
    return session.execute(
        sqlite_insert(Message)
        .values(
            owner_member_id=message_dict["owner_member_id"],
            from_member_id=message_dict["from_member_id"],
            to_member_id=message_dict["to_member_id"],
            type=message_dict["type"],
            created_at=message_dict["created_at"],
            status_state=message_dict["status_state"],
            status_timestamp=message_dict["status_timestamp"],
            origin_message_id=message_dict["origin_message_id"],
            text=message_dict["text"],
        )
        .returning(Message.message_id)
    ).scalar_one()


def _save_message(session, message_dict: dict) -> None:
    """UPDATE an existing message's mutable fields, keyed by ``message_id``."""
    session.execute(
        update(Message)
        .where(Message.message_id == message_dict["message_id"])
        .values(
            status_state=message_dict["status_state"],
            status_timestamp=message_dict["status_timestamp"],
            origin_message_id=message_dict["origin_message_id"],
            text=message_dict["text"],
        )
    )


def _unicast_message_dict(
    *,
    recipient_id: int,
    sender_id: int,
    text: str,
    now: str,
    origin_message_id: int | None = None,
) -> dict:
    return {
        "owner_member_id": recipient_id,
        "from_member_id": sender_id,
        "to_member_id": recipient_id,
        "type": "unicast",
        "created_at": now,
        "status_state": "input_required",
        "status_timestamp": now,
        "origin_message_id": origin_message_id,
        "text": text,
    }


def send_message(fleet_id: int, member_id: int, to: int | str, text: str) -> dict:
    """Create a unicast message addressed to ``to`` and best-effort notify it.

    Persists a new ``Message`` row with ``type='unicast'`` and
    ``status_state='input_required'``, then calls
    ``_try_notify_recipient`` to keystroke an inline preview into the
    recipient's tmux pane. Notification failure does not roll back the
    insert — the message remains available via :func:`poll_messages`.

    Args:
        fleet_id: Fleet id; sender and recipient must both belong to it.
        member_id: Sender's member id.
        to: Recipient's member id. Accepts a string for non-CLI callers
            (WebUI, tests); it is coerced with ``int(...)``.
        text: Message body. Truncation is render-side; the persisted row
            holds the full string.

    Returns:
        Dict with ``message`` (the persisted message dict) and
        ``notification_sent`` (boolean indicating whether the inline-preview
        keystroke landed).

    Raises:
        ValueError: If ``to`` is not a valid integer, the sender is not
            active in ``fleet_id``, or the recipient is missing or lives in a
            different fleet.
    """
    try:
        to_id = int(to)
    except (TypeError, ValueError) as exc:
        raise ValueError(f"Invalid destination format: {to}") from exc

    with _shared.write_session() as session:
        if not _shared.member_is_active_in_fleet(session, member_id, fleet_id):
            raise ValueError(
                f"Sender member not found or not active in fleet: {member_id}"
            )

        dest_fleet = session.execute(
            select(Member.fleet_id).where(
                Member.member_id == to_id,
                Member.status == "active",
            )
        ).scalar_one_or_none()
        if dest_fleet is None:
            raise ValueError(f"Destination member not found: {to_id}")
        if dest_fleet != fleet_id:
            raise ValueError(f"Destination member not in fleet: {to_id}")

        message_dict = _unicast_message_dict(
            recipient_id=to_id,
            sender_id=member_id,
            text=text,
            now=_shared.now_iso(),
        )
        message_dict["message_id"] = _insert_message(session, message_dict)
        notification_sent = _try_notify_recipient(
            session,
            recipient_id=to_id,
            sender_id=member_id,
            message_dict=message_dict,
        )

    return {"message": message_dict, "notification_sent": notification_sent}


def broadcast_message(fleet_id: int, member_id: int, text: str) -> list[dict]:
    """Fan out one delivery message per active peer plus a sender summary.

    Every
    delivery row shares the same ``origin_message_id`` (the summary's message
    id) so receivers can thread back to the original broadcast.

    Args:
        fleet_id: Fleet id to scope the broadcast to.
        member_id: Broadcaster's member id.
        text: Message body delivered to every recipient.

    Returns:
        Single-element list containing a dict with ``message`` (the summary row
        owned by the broadcaster), ``recipients`` (the real fan-out count N),
        and ``delivered`` (the number of inline-preview keystrokes that landed
        successfully, k ≤ N).

    Raises:
        ValueError: If the sender is not active in ``fleet_id``.
    """
    with _shared.write_session() as session:
        if not _shared.member_is_active_in_fleet(session, member_id, fleet_id):
            raise ValueError(
                f"Sender member not found or not active in fleet: {member_id}"
            )

        recipient_ids = list(
            session.execute(
                select(Member.member_id).where(
                    Member.fleet_id == fleet_id,
                    Member.status == "active",
                    Member.member_id != member_id,
                )
            ).scalars()
        )

        now = _shared.now_iso()
        summary_dict = {
            "owner_member_id": member_id,
            "from_member_id": member_id,
            "to_member_id": None,
            "type": "broadcast_summary",
            "created_at": now,
            "status_state": "completed",
            "status_timestamp": now,
            "origin_message_id": None,
            "text": f"Broadcast sent to {len(recipient_ids)} recipients",
        }
        summary_message_id = _insert_message(session, summary_dict)
        summary_dict["message_id"] = summary_message_id
        summary_dict["origin_message_id"] = summary_message_id
        _save_message(session, summary_dict)

        deliveries: list[tuple[int, dict]] = []
        for recipient_id in recipient_ids:
            delivery_dict = _unicast_message_dict(
                recipient_id=recipient_id,
                sender_id=member_id,
                text=text,
                now=now,
                origin_message_id=summary_message_id,
            )
            delivery_dict["message_id"] = _insert_message(session, delivery_dict)
            deliveries.append((recipient_id, delivery_dict))

        notifications_sent_count = sum(
            _try_notify_recipient(
                session,
                recipient_id=recipient_id,
                sender_id=member_id,
                message_dict=delivery_dict,
            )
            for recipient_id, delivery_dict in deliveries
        )

    return [
        {
            "message": summary_dict,
            "recipients": len(recipient_ids),
            "delivered": notifications_sent_count,
        }
    ]


def poll_messages(member_id: int) -> list[dict]:
    """Return un-acked deliveries addressed to ``member_id``, newest first.

    Only ``input_required`` messages are returned — once a delivery is ACKed
    (``completed``) it no longer appears. ``broadcast_summary``
    rows are filtered out, as those belong to the broadcaster's own inbox
    and are not deliveries.

    Args:
        member_id: Recipient member id; matches ``Message.owner_member_id``.

    Returns:
        List of flat message dicts (one per row) carrying every column from the
        ``messages`` table, in DESC ``status_timestamp`` order.
    """
    return _shared.list_messages_where(
        Message.owner_member_id == member_id,
        status="input_required",
    )


def ack_message(member_id: int, message_id: int) -> dict:
    """Transition a message from ``input_required`` to ``completed`` for the recipient.

    Args:
        member_id: Recipient member id; must match ``Message.owner_member_id``.
        message_id: Message id to ack.

    Returns:
        Dict with ``message`` — the updated message dict.

    Raises:
        ValueError: If the message does not exist or is not in
            ``input_required`` state.
        PermissionError: If ``member_id`` is not the recipient.
    """
    with _shared.write_session() as session:
        message_dict = _shared.read_message(session, message_id)
        if message_dict is None:
            raise ValueError(f"Message {message_id} not found")

        if message_dict["owner_member_id"] != member_id:
            raise PermissionError("Only the recipient can ACK a message")

        if message_dict["status_state"] != "input_required":
            raise ValueError(
                f"Cannot ACK message in state {message_dict['status_state']}"
            )

        message_dict["status_state"] = "completed"
        message_dict["status_timestamp"] = _shared.now_iso()

        _save_message(session, message_dict)

    return {"message": message_dict}
