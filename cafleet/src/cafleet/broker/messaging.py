"""Message send/broadcast/poll/ack/cancel and inline-preview notification."""

from sqlalchemy import select, update
from sqlalchemy.dialects.sqlite import insert as sqlite_insert

from cafleet.broker import _shared
from cafleet.config import settings
from cafleet.db.models import Agent, AgentPlacement, Task


def _try_notify_recipient(
    session, *, recipient_id: int, sender_id: int, task_dict: dict
) -> bool:
    """Best-effort inline-preview keystroke for the recipient's pane.

    Keystrokes a 2-line preview of the message itself into the recipient's
    pane — the recipient's TUI processes the keystrokes as a fresh user-turn
    input and the recipient acks via
    ``cafleet message ack --task-id <id>``. The queue remains the source
    of truth; failures are swallowed.
    """
    if recipient_id == sender_id:
        return False
    pane_id = session.execute(
        select(AgentPlacement.tmux_pane_id).where(
            AgentPlacement.agent_id == recipient_id
        )
    ).scalar_one_or_none()
    if pane_id is None:
        return False
    # Local import so tests that monkeypatch
    # ``cafleet.multiplexer.tmux.TmuxMultiplexer.send_inline_preview``
    # get picked up on every call rather than bound once at broker import.
    from cafleet.multiplexer.tmux import TmuxMultiplexer

    # Truncate before keystroking so a multi-KB body cannot dump itself into
    # the recipient's pane. Mirrors output.truncate_text's contract: same
    # limit (``settings.max_text_len`` / ``CAFLEET_MAX_TEXT_LEN``, default
    # 200) and same single-codepoint U+2026 suffix on overflow.
    preview_text = task_dict["text"]
    if len(preview_text) > settings.max_text_len:
        preview_text = preview_text[: settings.max_text_len] + "…"

    return TmuxMultiplexer().send_inline_preview(
        target_pane_id=pane_id,
        task_id=task_dict["task_id"],
        sender_id=sender_id,
        ts=task_dict["status_timestamp"],
        text=preview_text,
    )


def _insert_task(session, task_dict: dict) -> int:
    """INSERT a new task without a ``task_id`` and return the DB-assigned id."""
    return session.execute(
        sqlite_insert(Task)
        .values(
            context_id=task_dict["context_id"],
            from_agent_id=task_dict["from_agent_id"],
            to_agent_id=task_dict["to_agent_id"],
            type=task_dict["type"],
            created_at=task_dict["created_at"],
            status_state=task_dict["status_state"],
            status_timestamp=task_dict["status_timestamp"],
            origin_task_id=task_dict["origin_task_id"],
            text=task_dict["text"],
        )
        .returning(Task.task_id)
    ).scalar_one()


def _save_task(session, task_dict: dict) -> None:
    """UPDATE an existing task's mutable fields, keyed by ``task_id``."""
    session.execute(
        update(Task)
        .where(Task.task_id == task_dict["task_id"])
        .values(
            status_state=task_dict["status_state"],
            status_timestamp=task_dict["status_timestamp"],
            origin_task_id=task_dict["origin_task_id"],
            text=task_dict["text"],
        )
    )


def _unicast_task_dict(
    *,
    recipient_id: int,
    sender_id: int,
    text: str,
    now: str,
    origin_task_id: int | None = None,
) -> dict:
    return {
        "context_id": recipient_id,
        "from_agent_id": sender_id,
        "to_agent_id": recipient_id,
        "type": "unicast",
        "created_at": now,
        "status_state": "input_required",
        "status_timestamp": now,
        "origin_task_id": origin_task_id,
        "text": text,
    }


def send_message(fleet_id: int, agent_id: int, to: int | str, text: str) -> dict:
    """Create a unicast task addressed to ``to`` and best-effort notify it.

    Persists a new ``Task`` row with ``type='unicast'`` and
    ``status_state='input_required'``, then calls
    ``_try_notify_recipient`` to keystroke an inline preview into the
    recipient's tmux pane. Notification failure does not roll back the
    insert — the message remains available via :func:`poll_tasks`.

    Args:
        fleet_id: Fleet id; sender and recipient must both belong to it.
        agent_id: Sender's agent id.
        to: Recipient's agent id. Accepts a string for non-CLI callers
            (WebUI, tests); it is coerced with ``int(...)``.
        text: Message body. Truncation is render-side; the persisted row
            holds the full string.

    Returns:
        Dict with ``task`` (the persisted task dict) and ``notification_sent``
        (boolean indicating whether the inline-preview keystroke landed).

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
        if not _shared.agent_is_active_in_fleet(session, agent_id, fleet_id):
            raise ValueError(
                f"Sender agent not found or not active in fleet: {agent_id}"
            )

        dest_fleet = session.execute(
            select(Agent.fleet_id).where(
                Agent.agent_id == to_id,
                Agent.status == "active",
            )
        ).scalar_one_or_none()
        if dest_fleet is None:
            raise ValueError(f"Destination agent not found: {to_id}")
        if dest_fleet != fleet_id:
            raise ValueError(f"Destination agent not in fleet: {to_id}")

        task_dict = _unicast_task_dict(
            recipient_id=to_id,
            sender_id=agent_id,
            text=text,
            now=_shared.now_iso(),
        )
        task_dict["task_id"] = _insert_task(session, task_dict)
        notification_sent = _try_notify_recipient(
            session,
            recipient_id=to_id,
            sender_id=agent_id,
            task_dict=task_dict,
        )

    return {"task": task_dict, "notification_sent": notification_sent}


def broadcast_message(fleet_id: int, agent_id: int, text: str) -> list[dict]:
    """Fan out one delivery task per active non-admin peer plus a sender summary.

    Administrators are excluded at the SQL layer via ``json_extract`` so the
    card blob stays in the database; they are write-only identities. Every
    delivery row shares the same ``origin_task_id`` (the summary's task id)
    so receivers can thread back to the original broadcast.

    Args:
        fleet_id: Fleet id to scope the broadcast to.
        agent_id: Broadcaster's agent id.
        text: Message body delivered to every recipient.

    Returns:
        Single-element list containing a dict with ``task`` (the summary row
        owned by the broadcaster), ``recipients`` (the real fan-out count N),
        and ``delivered`` (the number of inline-preview keystrokes that landed
        successfully, k ≤ N).

    Raises:
        ValueError: If the sender is not active in ``fleet_id``.
    """
    with _shared.write_session() as session:
        if not _shared.agent_is_active_in_fleet(session, agent_id, fleet_id):
            raise ValueError(
                f"Sender agent not found or not active in fleet: {agent_id}"
            )

        recipient_ids = list(
            session.execute(
                select(Agent.agent_id).where(
                    Agent.fleet_id == fleet_id,
                    Agent.status == "active",
                    Agent.agent_id != agent_id,
                    _shared.CARD_KIND_SQL != _shared.ADMINISTRATOR_KIND,
                )
            ).scalars()
        )

        now = _shared.now_iso()
        summary_dict = {
            "context_id": agent_id,
            "from_agent_id": agent_id,
            "to_agent_id": None,
            "type": "broadcast_summary",
            "created_at": now,
            "status_state": "completed",
            "status_timestamp": now,
            "origin_task_id": None,
            "text": f"Broadcast sent to {len(recipient_ids)} recipients",
        }
        summary_task_id = _insert_task(session, summary_dict)
        summary_dict["task_id"] = summary_task_id
        summary_dict["origin_task_id"] = summary_task_id
        _save_task(session, summary_dict)

        deliveries: list[tuple[int, dict]] = []
        for recipient_id in recipient_ids:
            delivery_dict = _unicast_task_dict(
                recipient_id=recipient_id,
                sender_id=agent_id,
                text=text,
                now=now,
                origin_task_id=summary_task_id,
            )
            delivery_dict["task_id"] = _insert_task(session, delivery_dict)
            deliveries.append((recipient_id, delivery_dict))

        notifications_sent_count = sum(
            _try_notify_recipient(
                session,
                recipient_id=recipient_id,
                sender_id=agent_id,
                task_dict=delivery_dict,
            )
            for recipient_id, delivery_dict in deliveries
        )

    return [
        {
            "task": summary_dict,
            "recipients": len(recipient_ids),
            "delivered": notifications_sent_count,
        }
    ]


def poll_tasks(agent_id: int) -> list[dict]:
    """Return un-acked deliveries addressed to ``agent_id``, newest first.

    Only ``input_required`` tasks are returned — once a delivery is ACKed
    (``completed``) or canceled it no longer appears. ``broadcast_summary``
    rows are filtered out, as those belong to the broadcaster's own context
    and are not deliveries.

    Args:
        agent_id: Recipient agent id; matches ``Task.context_id``.

    Returns:
        List of flat task dicts (one per row) carrying every column from the
        ``tasks`` table, in DESC ``status_timestamp`` order.
    """
    return _shared.list_tasks_where(
        Task.context_id == agent_id,
        status="input_required",
    )


def _transition_task_state(
    agent_id: int,
    task_id: int,
    *,
    expected_agent_field: str,
    new_state: str,
    action_verb: str,
    permission_error_msg: str,
) -> dict:
    with _shared.write_session() as session:
        task_dict = _shared.read_task(session, task_id)
        if task_dict is None:
            raise ValueError(f"Task {task_id} not found")

        if task_dict[expected_agent_field] != agent_id:
            raise PermissionError(permission_error_msg)

        if task_dict["status_state"] != "input_required":
            raise ValueError(
                f"Cannot {action_verb} task in state {task_dict['status_state']}"
            )

        task_dict["status_state"] = new_state
        task_dict["status_timestamp"] = _shared.now_iso()

        _save_task(session, task_dict)

    return {"task": task_dict}


def ack_task(agent_id: int, task_id: int) -> dict:
    """Transition a task from ``input_required`` to ``completed`` for the recipient.

    Args:
        agent_id: Recipient agent id; must match ``Task.context_id``.
        task_id: Task id to ack.

    Returns:
        Dict with ``task`` — the updated task dict.

    Raises:
        ValueError: If the task does not exist or is not in
            ``input_required`` state.
        PermissionError: If ``agent_id`` is not the recipient.
    """
    return _transition_task_state(
        agent_id,
        task_id,
        expected_agent_field="context_id",
        new_state="completed",
        action_verb="ACK",
        permission_error_msg="Only the recipient can ACK a task",
    )


def cancel_task(agent_id: int, task_id: int) -> dict:
    """Transition a task from ``input_required`` to ``canceled`` for the sender.

    Args:
        agent_id: Sender agent id; must match ``Task.from_agent_id``.
        task_id: Task id to cancel.

    Returns:
        Dict with ``task`` — the updated task dict.

    Raises:
        ValueError: If the task does not exist or is not in
            ``input_required`` state.
        PermissionError: If ``agent_id`` is not the sender.
    """
    return _transition_task_state(
        agent_id,
        task_id,
        expected_agent_field="from_agent_id",
        new_state="canceled",
        action_verb="cancel",
        permission_error_msg="Only the sender can cancel a task",
    )
