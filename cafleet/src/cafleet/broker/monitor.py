"""Monitor schedule + runtime DB layer.

The pure data-access half of ``cafleet monitor`` (see
``docs/concepts/monitoring.md``): per-member schedule CRUD (``monitor_config``),
the per-tick scan, ping recording, and the single-instance runtime
claim/heartbeat/clear (``monitor_runtime``) with an ownership-checked
split-brain guard. No OS side effects live here — the loop and its signal
handling belong to the ``cafleet.monitor`` package.

The ``monitor_config.enabled`` column is an ``INTEGER`` 0/1, but every read
function casts it to a Python ``bool`` at the boundary, so the integer
representation never leaks past the broker.
"""

import hashlib
import os
import re
import secrets
from datetime import UTC, datetime, timedelta

import click
from sqlalchemy import delete, func, select, update

from cafleet.broker import _shared
from cafleet.config import settings
from cafleet.db.models import (
    Fleet,
    Member,
    MemberPlacement,
    Message,
    MonitorConfig,
    MonitorDirectorGate,
    MonitorReportDelivery,
    MonitorRuntime,
)

# Role-based enrollment intervals and the liveness window. The ``cafleet.monitor``
# package imports these so each value has a single home in the layer that computes
# liveness; ``STALE_AFTER = max(FACTOR * tick_seconds, FLOOR_SECONDS)``. The root
# Director is enrolled at ``DIRECTOR_PING_INTERVAL_SECONDS`` and every ordinary
# member at ``MEMBER_PING_INTERVAL_SECONDS``; ``enroll_member`` takes the interval
# explicitly, so no single implicit default is shared across the two roles.
DIRECTOR_PING_INTERVAL_SECONDS = 180
MEMBER_PING_INTERVAL_SECONDS = 720
MONITOR_STALE_FACTOR = 3
MONITOR_STALE_FLOOR_SECONDS = 15


def _config_dict(row) -> dict:
    return {
        "member_id": row.member_id,
        "interval_seconds": row.interval_seconds,
        "last_ping_at": row.last_ping_at,
        "enabled": bool(row.enabled),
        "last_stall_check_at": row.last_stall_check_at,
        "last_stall_candidate_at": row.last_stall_candidate_at,
        "last_stall_capture_sha256": row.last_stall_capture_sha256,
        "stall_episode_state": row.stall_episode_state,
        "stall_escalation_reason": row.stall_escalation_reason,
    }


_CONFIG_COLS = (
    MonitorConfig.member_id,
    MonitorConfig.interval_seconds,
    MonitorConfig.last_ping_at,
    MonitorConfig.enabled,
    MonitorConfig.last_stall_check_at,
    MonitorConfig.last_stall_candidate_at,
    MonitorConfig.last_stall_capture_sha256,
    MonitorConfig.stall_episode_state,
    MonitorConfig.stall_escalation_reason,
)


def enroll_member(session, member_id: int, interval: int) -> None:
    """Insert a ``monitor_config`` row for a watched member at ``interval``.

    Called inside the same write transaction as the member/placement insert, so
    enrollment is atomic with registration. Enrollment covers the watched set:
    the root Director (``create_fleet``, ``DIRECTOR_PING_INTERVAL_SECONDS``) and
    every pane-bound ordinary member (``register_member``,
    ``MEMBER_PING_INTERVAL_SECONDS``). The dedicated monitoring member is the
    unenrolled watcher (located by kind) and is not enrolled here; ``interval``
    is required so each role's cadence is passed explicitly.
    """
    session.add(
        MonitorConfig(member_id=member_id, interval_seconds=interval, enabled=1)
    )


def find_monitoring_member(fleet_id: int) -> dict | None:
    """Return the fleet's active monitoring member as ``{member_id, name, pane_id}``.

    The monitoring member is the unenrolled *watcher*, identified by
    ``member_card_json.cafleet.kind == MONITORING_MEMBER_KIND`` and inner-joined
    to ``member_placements`` for its pane. There is at most one active per fleet
    (the ``register_member`` guard), so the loop locates it by kind rather than
    by a ``monitor_config`` row. The pane must be bound (non-NULL
    ``mux_pane_id``): a monitoring member whose placement is still pending has
    no wakeable pane, so it is treated as absent. Returns ``None`` when the
    fleet has no active, pane-bound monitoring member.
    """
    stmt = (
        select(Member.member_id, Member.name, MemberPlacement.mux_pane_id)
        .join(MemberPlacement, MemberPlacement.member_id == Member.member_id)
        .where(
            Member.fleet_id == fleet_id,
            Member.status == "active",
            MemberPlacement.mux_pane_id.is_not(None),
            _shared.CARD_KIND_SQL == _shared.MONITORING_MEMBER_KIND,
        )
    )
    with _shared.read_session() as session:
        row = session.execute(stmt).first()
    if row is None:
        return None
    return {
        "member_id": row.member_id,
        "name": row.name,
        "pane_id": row.mux_pane_id,
    }


def get_monitor_config(fleet_id: int, member_id: int) -> dict | None:
    """Return the member's schedule, or ``None`` if not enrolled / not in fleet."""
    stmt = (
        select(*_CONFIG_COLS)
        .join(Member, Member.member_id == MonitorConfig.member_id)
        .where(MonitorConfig.member_id == member_id, Member.fleet_id == fleet_id)
    )
    with _shared.read_session() as session:
        row = session.execute(stmt).first()
    return _config_dict(row) if row is not None else None


def list_monitor_configs(fleet_id: int) -> list[dict]:
    """Return every enrolled member's schedule in the fleet (``enabled`` as bool)."""
    stmt = (
        select(*_CONFIG_COLS)
        .join(Member, Member.member_id == MonitorConfig.member_id)
        .where(Member.fleet_id == fleet_id)
    )
    with _shared.read_session() as session:
        rows = session.execute(stmt).all()
    return [_config_dict(row) for row in rows]


def _apply_nonlive_episode_cleanup(config: MonitorConfig) -> None:
    """Apply the shared disable/dead/pending-placement episode transition."""
    config.last_stall_check_at = None
    if config.stall_episode_state == "nudge_claimed":
        config.stall_episode_state = "escalation_pending"
        config.stall_escalation_reason = "ping_interrupted"
        return
    if config.stall_episode_state == "escalation_pending":
        return
    config.last_stall_candidate_at = None
    config.last_stall_capture_sha256 = None
    config.stall_episode_state = "clear"
    config.stall_escalation_reason = None


def update_monitor_config(
    fleet_id: int,
    member_id: int,
    *,
    interval_seconds: int | None = None,
    enabled: bool | None = None,
) -> dict:
    """Update an enrolled member's interval and/or enabled flag; return the new config.

    A partial update leaves the unspecified field untouched. ``enabled`` is
    accepted as a bool and written as 0/1.

    Raises:
        click.ClickException: If the member is not in the fleet or not enrolled.
    """
    with _shared.write_session() as session:
        enrolled = session.execute(
            select(MonitorConfig.member_id)
            .join(Member, Member.member_id == MonitorConfig.member_id)
            .where(MonitorConfig.member_id == member_id, Member.fleet_id == fleet_id)
        ).scalar_one_or_none()
        if enrolled is None:
            raise click.ClickException(
                f"member {member_id} is not enrolled in monitoring "
                f"for fleet {fleet_id}."
            )

        config = session.get(MonitorConfig, member_id)
        if config is None:  # Defensive: enrollment was verified in this transaction.
            raise click.ClickException(
                f"member {member_id} is not enrolled in monitoring "
                f"for fleet {fleet_id}."
            )
        if interval_seconds is not None:
            config.interval_seconds = interval_seconds
        if enabled is not None:
            config.enabled = 1 if enabled else 0
            if not enabled:
                _apply_nonlive_episode_cleanup(config)
                session.execute(
                    delete(MonitorDirectorGate).where(
                        MonitorDirectorGate.director_member_id == member_id,
                        MonitorDirectorGate.fleet_id == fleet_id,
                    )
                )

        session.flush()
        row = session.execute(
            select(*_CONFIG_COLS).where(MonitorConfig.member_id == member_id)
        ).first()
        if row is None:
            raise click.ClickException(
                f"member {member_id} is not enrolled in monitoring "
                f"for fleet {fleet_id}."
            )
        return _config_dict(row)


def record_pings(member_ids: list[int], when: str) -> None:
    """Stamp ``last_ping_at`` for every member in one write transaction.

    Lets a tick's dispatched pings be recorded with a single ``UPDATE … WHERE
    member_id IN (…)`` instead of one transaction per member. ``when`` is an
    ISO-8601 string stored verbatim in the TEXT column. An empty list is a
    no-op (no transaction, no ``IN ()``).
    """
    if not member_ids:
        return
    with _shared.write_session() as session:
        session.execute(
            update(MonitorConfig)
            .where(MonitorConfig.member_id.in_(member_ids))
            .values(last_ping_at=when)
        )


def record_monitor_dispatch(
    ping_member_ids: list[int],
    stall_check_member_ids: list[int],
    when: str,
) -> None:
    """Atomically commit the interval and durable stall dispatch cadences."""
    if not ping_member_ids and not stall_check_member_ids:
        return
    with _shared.write_session() as session:
        if ping_member_ids:
            session.execute(
                update(MonitorConfig)
                .where(MonitorConfig.member_id.in_(ping_member_ids))
                .values(last_ping_at=when)
            )
        if stall_check_member_ids:
            session.execute(
                update(MonitorConfig)
                .where(MonitorConfig.member_id.in_(stall_check_member_ids))
                .values(last_stall_check_at=when)
            )


def reconcile_monitor_lifecycle(
    fleet_id: int, unavailable_member_ids: list[int]
) -> None:
    """Batch-clean disabled, placement-pending, and dead watched members."""
    if not unavailable_member_ids:
        return
    member_ids = sorted(set(unavailable_member_ids))
    with _shared.write_session() as session:
        rows = session.execute(
            select(MonitorConfig, Fleet.director_member_id)
            .join(Member, Member.member_id == MonitorConfig.member_id)
            .join(Fleet, Fleet.fleet_id == Member.fleet_id)
            .where(
                Member.fleet_id == fleet_id,
                Member.member_id.in_(member_ids),
                Member.status == "active",
            )
        ).all()
        director_unavailable = False
        for config, director_member_id in rows:
            _apply_nonlive_episode_cleanup(config)
            if config.member_id == director_member_id:
                director_unavailable = True
        if director_unavailable:
            session.execute(
                delete(MonitorDirectorGate).where(
                    MonitorDirectorGate.fleet_id == fleet_id
                )
            )


def list_monitor_targets(fleet_id: int) -> list[dict]:
    """Per-tick scan: one row per active, enrolled member — the watched set.

    Enrollment covers the root Director (180 s) and every ordinary member
    (720 s), so this returns those rows; the dedicated monitoring member is the
    unenrolled watcher and never appears here (it is located separately by
    ``find_monitoring_member``). Each dict carries ``member_id``, ``name``,
    ``is_director`` (derived from ``fleets.director_member_id``, used for the
    ``monitor status`` role label), ``pane_id``, ``interval_seconds``,
    ``last_ping_at``, ``enabled`` (bool), ``pending_count`` — the count of
    the member's ``input_required`` deliveries excluding ``broadcast_summary``
    rows, a correlated subquery mirroring ``members.py`` — and
    ``oldest_pending_ts`` — ``MIN(status_timestamp)`` over the same predicate
    set (a second correlated subquery; ``None`` when the member has no pending
    delivery).
    """
    pending_sq = (
        select(func.count(Message.message_id))
        .where(
            Message.owner_member_id == Member.member_id,
            Message.status_state == "input_required",
            _shared.NOT_BROADCAST_SUMMARY,
        )
        .correlate(Member)
        .scalar_subquery()
    )
    oldest_pending_sq = (
        select(func.min(Message.status_timestamp))
        .where(
            Message.owner_member_id == Member.member_id,
            Message.status_state == "input_required",
            _shared.NOT_BROADCAST_SUMMARY,
        )
        .correlate(Member)
        .scalar_subquery()
    )
    stmt = (
        select(
            Member.member_id,
            Member.name,
            MemberPlacement.mux_pane_id,
            MemberPlacement.coding_agent,
            Fleet.director_member_id,
            MonitorConfig.interval_seconds,
            MonitorConfig.last_ping_at,
            MonitorConfig.enabled,
            MonitorConfig.last_stall_check_at,
            pending_sq.label("pending_count"),
            oldest_pending_sq.label("oldest_pending_ts"),
        )
        .join(MonitorConfig, MonitorConfig.member_id == Member.member_id)
        .join(MemberPlacement, MemberPlacement.member_id == Member.member_id)
        .join(Fleet, Fleet.fleet_id == Member.fleet_id)
        .where(Member.fleet_id == fleet_id, Member.status == "active")
    )
    with _shared.read_session() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "member_id": row.member_id,
            "name": row.name,
            "is_director": row.member_id == row.director_member_id,
            "pane_id": row.mux_pane_id,
            "coding_agent": row.coding_agent,
            "interval_seconds": row.interval_seconds,
            "last_ping_at": row.last_ping_at,
            "enabled": bool(row.enabled),
            "last_stall_check_at": row.last_stall_check_at,
            "pending_count": row.pending_count,
            "oldest_pending_ts": row.oldest_pending_ts,
        }
        for row in rows
    ]


_CAPTURE_SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")
_STALL_CLASSIFICATIONS = {
    "awaiting_user",
    "unknown",
    "finished",
    "working",
    "stall_candidate",
}
_OPEN_PREVIEW_STATES = ("pending", "awaiting_ack")


def _clear_stall_episode(
    config: MonitorConfig, *, clear_candidate: bool = True
) -> None:
    if clear_candidate:
        config.last_stall_candidate_at = None
        config.last_stall_capture_sha256 = None
    config.stall_episode_state = "clear"
    config.stall_escalation_reason = None


def _stall_result(
    config: MonitorConfig,
    *,
    classification: str,
    action: str = "none",
    director_gate_token: str | None = None,
) -> dict:
    return {
        "member_id": config.member_id,
        "classification": classification,
        "action": action,
        "episode_state": config.stall_episode_state,
        "escalation_reason": config.stall_escalation_reason,
        "director_gate_token": director_gate_token,
    }


def _parse_capture_identity(
    classification: str,
    captured_at: str | None,
    content_sha256: str | None,
    *,
    now: datetime,
) -> datetime | None:
    if classification not in _STALL_CLASSIFICATIONS:
        raise click.ClickException(
            f"invalid monitor stall classification: {classification}"
        )
    if (captured_at is None) != (content_sha256 is None):
        raise click.ClickException(
            "captured_at and content_sha256 must be provided together"
        )
    if captured_at is None:
        if classification != "unknown":
            raise click.ClickException(
                "readable monitor observations require capture identity"
            )
        return None
    if classification == "unknown":
        raise click.ClickException(
            "loss-tolerant unknown observations must omit capture identity"
        )
    if not _CAPTURE_SHA256_RE.fullmatch(content_sha256 or ""):
        raise click.ClickException(
            "content_sha256 must be 64 lowercase hexadecimal characters"
        )
    try:
        parsed = datetime.fromisoformat(captured_at)
    except ValueError as exc:
        raise click.ClickException(
            "captured_at must be a timezone-aware UTC ISO-8601 timestamp"
        ) from exc
    if parsed.tzinfo is None or parsed.utcoffset() != timedelta(0):
        raise click.ClickException(
            "captured_at must be a timezone-aware UTC ISO-8601 timestamp"
        )
    if parsed > now:
        raise click.ClickException("captured_at cannot be in the future")
    return parsed


def _stall_target(
    session,
    fleet_id: int,
    member_id: int,
    *,
    director_gate: bool,
    require_live: bool,
):
    row = session.execute(
        select(
            Member,
            MonitorConfig,
            MemberPlacement.mux_pane_id,
            Fleet.director_member_id,
            _shared.CARD_KIND_SQL.label("card_kind"),
        )
        .join(Fleet, Fleet.fleet_id == Member.fleet_id)
        .join(MonitorConfig, MonitorConfig.member_id == Member.member_id)
        .outerjoin(MemberPlacement, MemberPlacement.member_id == Member.member_id)
        .where(
            Member.fleet_id == fleet_id,
            Member.member_id == member_id,
            Member.status == "active",
            Fleet.deleted_at.is_(None),
        )
    ).first()
    if row is None:
        raise click.ClickException(
            f"member {member_id} is not an active enrolled member of fleet {fleet_id}"
        )
    is_director = member_id == row.director_member_id
    is_monitor = row.card_kind == _shared.MONITORING_MEMBER_KIND
    if director_gate:
        if not is_director:
            raise click.ClickException(
                f"member {member_id} is not the active root Director"
            )
    elif is_director or is_monitor:
        raise click.ClickException(
            f"member {member_id} is not an eligible ordinary monitor target"
        )
    if require_live and (
        not bool(row.MonitorConfig.enabled) or row.mux_pane_id is None
    ):
        raise click.ClickException(
            f"member {member_id} does not have an enabled live placement"
        )
    return row


def _issue_director_gate(
    session,
    *,
    fleet_id: int,
    director_member_id: int,
    classification: str,
    now: datetime,
) -> str:
    raw_token = secrets.token_bytes(32)
    token = raw_token.hex()
    issued_at = now.isoformat()
    session.add(
        MonitorDirectorGate(
            fleet_id=fleet_id,
            director_member_id=director_member_id,
            token_sha256=hashlib.sha256(raw_token).hexdigest(),
            classification=classification,
            issued_at=issued_at,
            expires_at=(now + timedelta(seconds=30)).isoformat(),
        )
    )
    return token


def _observe_director_candidate(
    config: MonitorConfig,
    *,
    captured_time: datetime,
    captured_at: str,
    content_sha256: str,
) -> str:
    if settings.monitor_stall_interval <= 0:
        config.last_stall_candidate_at = None
        config.last_stall_capture_sha256 = None
        return "unknown"
    if config.last_stall_candidate_at is None:
        config.last_stall_candidate_at = captured_at
        config.last_stall_capture_sha256 = content_sha256
        return "unknown"
    previous = datetime.fromisoformat(config.last_stall_candidate_at)
    elapsed = (captured_time - previous).total_seconds()
    if captured_time <= previous or elapsed < settings.monitor_stall_interval:
        return "unknown"
    changed = config.last_stall_capture_sha256 != content_sha256
    config.last_stall_candidate_at = captured_at
    config.last_stall_capture_sha256 = content_sha256
    return "working" if changed else "stalled"


def _observe_ordinary_candidate(
    config: MonitorConfig,
    *,
    captured_time: datetime,
    captured_at: str,
    content_sha256: str,
    stall_check: bool,
) -> tuple[str, str]:
    state = config.stall_episode_state
    previous_at = (
        datetime.fromisoformat(config.last_stall_candidate_at)
        if config.last_stall_candidate_at is not None
        else None
    )

    if state == "escalation_pending":
        return "unknown", "escalate"

    if state in {"nudge_claimed", "nudged", "escalated"}:
        if previous_at is None or captured_time <= previous_at:
            return "unknown", "none"
        if config.last_stall_capture_sha256 == content_sha256:
            if state == "nudge_claimed":
                config.stall_episode_state = "escalation_pending"
                config.stall_escalation_reason = "ping_interrupted"
                return "stalled", "escalate"
            if state == "nudged":
                config.stall_episode_state = "escalation_pending"
                config.stall_escalation_reason = "unchanged_after_nudge"
                return "stalled", "escalate"
            config.last_stall_candidate_at = captured_at
            return "stalled", "none"
        _clear_stall_episode(config, clear_candidate=not stall_check)
        if stall_check:
            config.last_stall_candidate_at = captured_at
            config.last_stall_capture_sha256 = content_sha256
        return "working", "none"

    if not stall_check or settings.monitor_stall_interval <= 0:
        return "unknown", "none"
    if previous_at is None:
        config.last_stall_candidate_at = captured_at
        config.last_stall_capture_sha256 = content_sha256
        return "unknown", "none"
    elapsed = (captured_time - previous_at).total_seconds()
    if captured_time <= previous_at or elapsed < settings.monitor_stall_interval:
        return "unknown", "none"
    if config.last_stall_capture_sha256 != content_sha256:
        config.last_stall_candidate_at = captured_at
        config.last_stall_capture_sha256 = content_sha256
        return "working", "none"
    config.last_stall_candidate_at = captured_at
    config.last_stall_capture_sha256 = content_sha256
    config.stall_episode_state = "nudge_claimed"
    config.stall_escalation_reason = None
    return "stalled", "ping"


def observe_stall_episode(
    fleet_id: int,
    member_id: int,
    *,
    classification: str,
    captured_at: str | None = None,
    content_sha256: str | None = None,
    stall_check: bool = False,
    director_gate: bool = False,
) -> dict:
    """Atomically observe an ordinary stall episode or issue a Director gate."""
    if stall_check and director_gate:
        raise click.ClickException(
            "stall_check and director_gate are mutually exclusive"
        )
    now = datetime.now(UTC)
    captured_time = _parse_capture_identity(
        classification,
        captured_at,
        content_sha256,
        now=now,
    )
    with _shared.write_session() as session:
        row = _stall_target(
            session,
            fleet_id,
            member_id,
            director_gate=director_gate,
            require_live=captured_time is not None,
        )
        config = row.MonitorConfig

        if director_gate:
            session.execute(
                delete(MonitorDirectorGate).where(
                    MonitorDirectorGate.fleet_id == fleet_id
                )
            )
            resolved = classification
            if classification == "stall_candidate":
                assert captured_time is not None
                assert captured_at is not None
                assert content_sha256 is not None
                resolved = _observe_director_candidate(
                    config,
                    captured_time=captured_time,
                    captured_at=captured_at,
                    content_sha256=content_sha256,
                )
            elif classification == "unknown":
                config.last_stall_candidate_at = None
                config.last_stall_capture_sha256 = None
            else:
                _clear_stall_episode(config)

            token = None
            if resolved in {"finished", "stalled"}:
                token = _issue_director_gate(
                    session,
                    fleet_id=fleet_id,
                    director_member_id=member_id,
                    classification=resolved,
                    now=now,
                )
            return _stall_result(
                config,
                classification=resolved,
                director_gate_token=token,
            )

        if classification == "unknown":
            _apply_nonlive_episode_cleanup(config)
            action = (
                "escalate"
                if config.stall_episode_state == "escalation_pending"
                else "none"
            )
            return _stall_result(config, classification="unknown", action=action)

        if classification != "stall_candidate":
            if config.stall_episode_state == "escalation_pending":
                return _stall_result(
                    config, classification=classification, action="escalate"
                )
            _clear_stall_episode(config)
            return _stall_result(config, classification=classification)

        assert captured_time is not None
        assert captured_at is not None
        assert content_sha256 is not None
        resolved, action = _observe_ordinary_candidate(
            config,
            captured_time=captured_time,
            captured_at=captured_at,
            content_sha256=content_sha256,
            stall_check=stall_check,
        )
        return _stall_result(config, classification=resolved, action=action)


def record_stall_ping_result(
    fleet_id: int,
    member_id: int,
    *,
    success: bool,
) -> dict:
    """Record one claimed fixed-ping result without rechecking pane liveness."""
    with _shared.write_session() as session:
        row = _stall_target(
            session,
            fleet_id,
            member_id,
            director_gate=False,
            require_live=False,
        )
        config = row.MonitorConfig
        state = config.stall_episode_state
        reason = config.stall_escalation_reason
        if state == "nudge_claimed":
            if success:
                config.stall_episode_state = "nudged"
            else:
                config.stall_episode_state = "escalation_pending"
                config.stall_escalation_reason = "ping_failed"
        elif (state == "nudged" and success) or (
            state == "escalation_pending" and reason == "ping_failed" and not success
        ):
            pass
        elif state == "escalation_pending" and reason == "ping_interrupted":
            if not success:
                config.stall_escalation_reason = "ping_failed"
        else:
            raise click.ClickException(
                f"ping result conflicts with monitor stall episode "
                f"{state}/{reason or '-'}"
            )
        return {
            "member_id": member_id,
            "episode_state": config.stall_episode_state,
            "escalation_reason": config.stall_escalation_reason,
        }


def list_pending_stall_escalations(fleet_id: int) -> list[dict]:
    """Return durable pending ordinary-member escalations in stable order."""
    with _shared.read_session() as session:
        rows = session.execute(
            select(
                Member.member_id,
                Member.name,
                MonitorConfig.stall_escalation_reason,
            )
            .join(MonitorConfig, MonitorConfig.member_id == Member.member_id)
            .join(Fleet, Fleet.fleet_id == Member.fleet_id)
            .where(
                Member.fleet_id == fleet_id,
                Member.status == "active",
                Member.member_id != Fleet.director_member_id,
                MonitorConfig.stall_episode_state == "escalation_pending",
            )
            .order_by(Member.member_id)
        ).all()
    return [
        {
            "member_id": row.member_id,
            "name": row.name,
            "escalation_reason": row.stall_escalation_reason,
        }
        for row in rows
    ]


def _sanitize_monitor_report_name(name: str) -> str:
    return (
        name.replace("\r\n", "⏎")
        .replace("\n", "⏎")
        .replace("\r", "⏎")
        .replace("\t", "⏎")
        .replace("`", "ˋ")
        .replace("$(", "$﹙")
    )


def _report_entry(member_id: int, name: str, reason: str) -> str:
    safe_name = _sanitize_monitor_report_name(name)
    if reason == "ping_failed":
        detail = "direct inbox-poll nudge failed"
    elif reason == "ping_interrupted":
        detail = (
            "direct inbox-poll nudge outcome unknown before its result was recorded"
        )
    else:
        detail = "unchanged at next synchronized check after direct inbox-poll nudge"
    return f"monitor escalation: member {member_id} ({safe_name}) {detail}"


def _finished_entry(member_id: int, name: str) -> str:
    return (
        f"monitor finished: member {member_id} "
        f"({_sanitize_monitor_report_name(name)}) is at an empty input prompt; "
        "Director must decide whether assigned work remains"
    )


def _validate_director_gate_token(
    session,
    *,
    fleet_id: int,
    token: str,
    now: datetime,
):
    if not _CAPTURE_SHA256_RE.fullmatch(token):
        raise click.ClickException(
            "director gate token must be 64 lowercase hexadecimal characters"
        )
    expected = hashlib.sha256(bytes.fromhex(token)).hexdigest()
    current_director_id = (
        select(Fleet.director_member_id)
        .where(
            Fleet.fleet_id == fleet_id,
            Fleet.deleted_at.is_(None),
        )
        .scalar_subquery()
    )
    consumed = session.execute(
        delete(MonitorDirectorGate)
        .where(
            MonitorDirectorGate.fleet_id == fleet_id,
            MonitorDirectorGate.director_member_id == current_director_id,
            MonitorDirectorGate.token_sha256 == expected,
            MonitorDirectorGate.expires_at > now.isoformat(),
        )
        .returning(MonitorDirectorGate.director_member_id)
        .execution_options(synchronize_session=False)
    ).first()
    if consumed is None:
        raise click.ClickException(
            "monitor Director gate token is absent, mismatched, or expired"
        )

    fleet = session.execute(
        select(Fleet).where(
            Fleet.fleet_id == fleet_id,
            Fleet.deleted_at.is_(None),
        )
    ).scalar_one_or_none()
    if fleet is None or fleet.director_member_id is None:
        raise click.ClickException(f"fleet {fleet_id} has no active Director")
    director = session.execute(
        select(Member, MemberPlacement.mux_pane_id, MonitorConfig.interval_seconds)
        .join(MemberPlacement, MemberPlacement.member_id == Member.member_id)
        .join(MonitorConfig, MonitorConfig.member_id == Member.member_id)
        .where(
            Member.member_id == fleet.director_member_id,
            Member.fleet_id == fleet_id,
            Member.status == "active",
            MemberPlacement.mux_pane_id.is_not(None),
        )
    ).first()
    if director is None:
        raise click.ClickException("monitor report Director is unavailable")
    return fleet, director


def _find_monitoring_member_in_session(session, fleet_id: int):
    row = session.execute(
        select(Member.member_id, MemberPlacement.mux_pane_id)
        .join(MemberPlacement, MemberPlacement.member_id == Member.member_id)
        .where(
            Member.fleet_id == fleet_id,
            Member.status == "active",
            MemberPlacement.mux_pane_id.is_not(None),
            _shared.CARD_KIND_SQL == _shared.MONITORING_MEMBER_KIND,
        )
    ).first()
    if row is None:
        raise click.ClickException(f"fleet {fleet_id} has no active monitoring member")
    return row


def report_monitor_batch(
    fleet_id: int,
    *,
    director_gate_token: str,
    finished_member_ids: list[int],
) -> dict:
    """Consume a safe Director gate and reconcile/create/preview one aggregate."""
    now = datetime.now(UTC)
    now_iso = now.isoformat()
    preview_message: dict | None = None
    preview_recipient_id: int | None = None
    preview_sender_id: int | None = None
    result: dict[str, object] = {
        "created_message_id": None,
        "open_message_id": None,
        "preview_message_id": None,
        "escalated_member_ids": [],
        "finished_member_ids": [],
        "created": False,
        "preview_outcome": "none",
    }

    with _shared.write_session() as session:
        fleet, director = _validate_director_gate_token(
            session,
            fleet_id=fleet_id,
            token=director_gate_token,
            now=now,
        )
        watcher = _find_monitoring_member_in_session(session, fleet_id)

        open_delivery = session.execute(
            select(MonitorReportDelivery)
            .where(
                MonitorReportDelivery.fleet_id == fleet_id,
                MonitorReportDelivery.preview_state.in_(_OPEN_PREVIEW_STATES),
            )
            .order_by(MonitorReportDelivery.message_id)
        ).scalar_one_or_none()
        if open_delivery is not None:
            open_message = session.get(Message, open_delivery.message_id)
            if open_message is None or open_message.status_state != "input_required":
                open_delivery.preview_state = "delivered"
                open_delivery.delivered_at = now_iso
                open_delivery = None

        if open_delivery is None:
            escalation_rows = session.execute(
                select(Member, MonitorConfig)
                .join(MonitorConfig, MonitorConfig.member_id == Member.member_id)
                .where(
                    Member.fleet_id == fleet_id,
                    Member.status == "active",
                    Member.member_id != fleet.director_member_id,
                    MonitorConfig.stall_episode_state == "escalation_pending",
                )
                .order_by(Member.member_id)
            ).all()

            finished_ids = sorted(set(finished_member_ids))
            finished_rows: list[Member] = []
            for finished_id in finished_ids:
                finished = session.execute(
                    select(Member)
                    .join(
                        MonitorConfig,
                        MonitorConfig.member_id == Member.member_id,
                    )
                    .where(
                        Member.member_id == finished_id,
                        Member.fleet_id == fleet_id,
                        Member.status == "active",
                        Member.member_id != fleet.director_member_id,
                        _shared.CARD_KIND_SQL != _shared.MONITORING_MEMBER_KIND,
                    )
                ).scalar_one_or_none()
                if finished is None:
                    raise click.ClickException(
                        f"finished member {finished_id} is not an active "
                        "enrolled ordinary member"
                    )
                finished_rows.append(finished)

            if escalation_rows or finished_rows:
                entries = [
                    _report_entry(
                        member.member_id,
                        member.name,
                        config.stall_escalation_reason,
                    )
                    for member, config in escalation_rows
                ]
                entries.extend(
                    _finished_entry(member.member_id, member.name)
                    for member in finished_rows
                )
                message_dict = {
                    "owner_member_id": fleet.director_member_id,
                    "from_member_id": watcher.member_id,
                    "to_member_id": fleet.director_member_id,
                    "type": "unicast",
                    "created_at": now_iso,
                    "status_state": "input_required",
                    "status_timestamp": now_iso,
                    "origin_message_id": None,
                    "text": "monitor report batch:\n- " + "\n- ".join(entries),
                }
                from cafleet.broker.messaging import _insert_message

                message_id = _insert_message(session, message_dict)
                message_dict["message_id"] = message_id
                session.add(
                    MonitorReportDelivery(
                        message_id=message_id,
                        fleet_id=fleet_id,
                        preview_state="pending",
                        attempt_count=0,
                    )
                )
                for _member, config in escalation_rows:
                    config.stall_episode_state = "escalated"
                result.update(
                    {
                        "created_message_id": message_id,
                        "open_message_id": message_id,
                        "escalated_member_ids": [
                            member.member_id for member, _config in escalation_rows
                        ],
                        "finished_member_ids": [
                            member.member_id for member in finished_rows
                        ],
                        "created": True,
                    }
                )
                open_delivery = session.get(MonitorReportDelivery, message_id)
                preview_message = message_dict
        else:
            result["open_message_id"] = open_delivery.message_id

        if open_delivery is not None:
            result["open_message_id"] = open_delivery.message_id
            preview_due = open_delivery.preview_state == "pending"
            if (
                open_delivery.preview_state == "awaiting_ack"
                and open_delivery.last_attempt_at is not None
            ):
                elapsed = (
                    now - datetime.fromisoformat(open_delivery.last_attempt_at)
                ).total_seconds()
                preview_due = elapsed >= director.interval_seconds
            if preview_due and preview_message is None:
                message_row = session.get(Message, open_delivery.message_id)
                if message_row is not None:
                    preview_message = _shared.row_to_message_dict(message_row)

        if preview_message is not None:
            preview_recipient_id = fleet.director_member_id
            preview_sender_id = watcher.member_id

    if preview_message is None:
        return result

    assert preview_recipient_id is not None
    assert preview_sender_id is not None
    from cafleet.broker.messaging import _try_notify_recipient

    with _shared.read_session() as session:
        preview_sent = _try_notify_recipient(
            session,
            recipient_id=preview_recipient_id,
            sender_id=preview_sender_id,
            message_dict=preview_message,
        )
    attempted_at = _shared.now_iso()
    with _shared.write_session() as session:
        delivery = session.get(MonitorReportDelivery, preview_message["message_id"])
        if delivery is not None and delivery.preview_state in _OPEN_PREVIEW_STATES:
            delivery.attempt_count += 1
            delivery.last_attempt_at = attempted_at
            if preview_sent:
                delivery.preview_state = "awaiting_ack"

    result["preview_message_id"] = preview_message["message_id"]
    result["preview_outcome"] = "awaiting_ack" if preview_sent else "failed"
    return result


def _is_live(row, now: datetime) -> bool:
    """True iff the runtime row's owner is still alive.

    Heartbeat freshness is the authority — a process that died silently stops
    rewriting ``last_tick_at``, so a stale heartbeat reads as dead even though
    the PID may still resolve. ``os.kill(pid, 0)`` is a corroborating signal.
    """
    if row.pid is None or row.last_tick_at is None:
        return False
    stale_after = max(
        MONITOR_STALE_FACTOR * row.tick_seconds, MONITOR_STALE_FLOOR_SECONDS
    )
    elapsed = (now - datetime.fromisoformat(row.last_tick_at)).total_seconds()
    if elapsed > stale_after:
        return False
    try:
        os.kill(row.pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        # The PID resolves but is owned by another user — it exists, so alive.
        return True
    return True


def claim_monitor_runtime(
    fleet_id: int, pid: int, tick_seconds: int, when: str
) -> bool:
    """Atomically claim the fleet's single monitor slot.

    Inserts a fresh row, reclaims a stale one, or refuses a live one — all in
    one write transaction (SQLite's write lock serializes concurrent claims).

    Returns:
        ``True`` if this ``pid`` now owns the slot; ``False`` if a live monitor
        already holds it.
    """
    now = datetime.fromisoformat(when)
    with _shared.write_session() as session:
        row = session.get(MonitorRuntime, fleet_id)
        if row is None:
            session.add(
                MonitorRuntime(
                    fleet_id=fleet_id,
                    pid=pid,
                    started_at=when,
                    last_tick_at=when,
                    tick_seconds=tick_seconds,
                )
            )
            return True
        if _is_live(row, now):
            return False
        row.pid = pid
        row.started_at = when
        row.last_tick_at = when
        row.tick_seconds = tick_seconds
        return True


def heartbeat_monitor_runtime(fleet_id: int, pid: int, when: str) -> bool:
    """Ownership-checked heartbeat — rewrites ``last_tick_at`` iff ``pid`` owns the slot.

    Returns ``False`` when the slot was reclaimed by another instance (the
    ``WHERE pid=?`` matches zero rows), the signal the displaced monitor uses
    to self-terminate without pinging.
    """
    with _shared.write_session() as session:
        result = session.execute(
            update(MonitorRuntime)
            .where(MonitorRuntime.fleet_id == fleet_id, MonitorRuntime.pid == pid)
            .values(last_tick_at=when)
        )
        return result.rowcount == 1


def clear_monitor_runtime(fleet_id: int, pid: int) -> None:
    """Ownership-checked clear — nulls ``pid``/``started_at``/``last_tick_at`` iff ``pid`` owns the slot.

    A cleanly-stopped monitor leaves no residual ``started_at``, so `status`/the
    WebUI report a fully-stopped row. A non-owner clear matches zero rows and is
    a no-op, so a self-terminating loser never wipes the winner's row on exit.
    """
    with _shared.write_session() as session:
        session.execute(
            update(MonitorRuntime)
            .where(MonitorRuntime.fleet_id == fleet_id, MonitorRuntime.pid == pid)
            .values(pid=None, started_at=None, last_tick_at=None)
        )


def read_monitor_runtime(fleet_id: int) -> dict | None:
    """Return the fleet's runtime row, or ``None`` when no monitor ever claimed it."""
    with _shared.read_session() as session:
        row = session.get(MonitorRuntime, fleet_id)
        if row is None:
            return None
        return {
            "fleet_id": row.fleet_id,
            "pid": row.pid,
            "started_at": row.started_at,
            "last_tick_at": row.last_tick_at,
            "tick_seconds": row.tick_seconds,
        }


def monitor_is_live(fleet_id: int, now: datetime) -> bool:
    """Return True iff the fleet currently has a live monitor holding the slot.

    The advisory single-instance pre-check for ``monitor start`` (the atomic
    ``claim_monitor_runtime`` is the authoritative guard). Reuses ``_is_live``:
    heartbeat freshness is authoritative, ``os.kill(pid, 0)`` corroborates.
    """
    with _shared.read_session() as session:
        row = session.get(MonitorRuntime, fleet_id)
        if row is None:
            return False
        return _is_live(row, now)


def monitor_runtime_payload(fleet_id: int, now: datetime) -> dict:
    """Build the runtime-liveness dict shared by ``monitor status`` and GET /api/monitor.

    When the monitor is not live (no row, or a stale/cleared heartbeat), the
    process fields (``pid`` / ``started_at`` / ``last_tick_at`` /
    ``last_tick_age_seconds``) are ``null`` — a stale row never reports a
    lingering pid or start time.
    """
    row = read_monitor_runtime(fleet_id)
    if row is None or not monitor_is_live(fleet_id, now):
        return {
            "running": False,
            "pid": None,
            "tick_seconds": row["tick_seconds"] if row is not None else None,
            "last_tick_at": None,
            "last_tick_age_seconds": None,
            "started_at": None,
        }
    age = None
    if row["last_tick_at"] is not None:
        age = int((now - datetime.fromisoformat(row["last_tick_at"])).total_seconds())
    return {
        "running": True,
        "pid": row["pid"],
        "tick_seconds": row["tick_seconds"],
        "last_tick_at": row["last_tick_at"],
        "last_tick_age_seconds": age,
        "started_at": row["started_at"],
    }


def monitor_members_payload(fleet_id: int, now: datetime) -> list[dict]:
    """Build the per-member rows shared by ``monitor status`` and GET /api/monitor.

    One dict per ``list_monitor_targets`` row, so the CLI and API payloads
    cannot drift. ``last_ping_age_seconds`` and ``oldest_pending_age_seconds``
    are whole seconds (integer-truncated) against the single supplied ``now``,
    ``None`` when the source timestamp is ``None``.
    """

    def _age(ts: str | None) -> int | None:
        if ts is None:
            return None
        return int((now - datetime.fromisoformat(ts)).total_seconds())

    return [
        {
            "member_id": t["member_id"],
            "name": t["name"],
            "role": "director" if t["is_director"] else "member",
            "interval_seconds": t["interval_seconds"],
            "last_ping_at": t["last_ping_at"],
            "last_ping_age_seconds": _age(t["last_ping_at"]),
            "enabled": t["enabled"],
            "pending_count": t["pending_count"],
            "oldest_pending_ts": t["oldest_pending_ts"],
            "oldest_pending_age_seconds": _age(t["oldest_pending_ts"]),
        }
        for t in list_monitor_targets(fleet_id)
    ]


def delete_fleet_monitor_rows(session, fleet_id: int) -> None:
    """Delete all durable monitor state for one fleet.

    Called inside ``delete_fleet``'s transaction, mirroring the explicit
    ``member_placements`` cleanup.
    """
    members_in_fleet = select(Member.member_id).where(Member.fleet_id == fleet_id)
    session.execute(
        delete(MonitorReportDelivery).where(MonitorReportDelivery.fleet_id == fleet_id)
    )
    session.execute(
        delete(MonitorDirectorGate).where(MonitorDirectorGate.fleet_id == fleet_id)
    )
    session.execute(
        delete(MonitorConfig).where(MonitorConfig.member_id.in_(members_in_fleet))
    )
    session.execute(delete(MonitorRuntime).where(MonitorRuntime.fleet_id == fleet_id))


def delete_member_monitor_row(session, member_id: int) -> None:
    """Delete one member's ``monitor_config`` row, alongside its placement cleanup."""
    session.execute(delete(MonitorConfig).where(MonitorConfig.member_id == member_id))
