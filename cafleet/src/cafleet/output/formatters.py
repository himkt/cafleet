"""Human-readable text formatters."""

from collections.abc import Callable
from typing import Any

from cafleet.output.render import format_json, render_message, truncate_text


def format_message(message: dict, *, full: bool = False) -> str:
    """Render a message as text.

    ``full=False`` (default): 2-line compact render —
    line 1 is ``[<id> | from:<from> | <ts>]`` (with optional
    ``| kind:<kind>`` and ``| origin:<id>`` segments), line 2 is the body.

    ``full=True``: verbose labeled layout that exposes every typed column
    (``id``, ``state``, ``from``, ``to``, ``type``, ``text``).
    """
    if "message" in message and isinstance(message["message"], dict):
        message = message["message"]
    if not full:
        rendered = render_message(message, full=False)
        segments = [f"[{rendered['id']} | from:{rendered['from']} | {rendered['ts']}"]
        if "kind" in rendered:
            segments.append(f" | kind:{rendered['kind']}")
        if "origin" in rendered:
            segments.append(f" | origin:{rendered['origin']}")
        segments.append("]")
        line1 = "".join(segments)
        body = rendered.get("text", "")
        if body:
            return f"{line1}\n{body}"
        return line1
    lines = [
        f"  id:    {message['message_id']}",
        f"  state: {message['status_state']}",
        f"  from:  {message['from_member_id']}",
    ]
    if message.get("to_member_id") is not None:
        lines.append(f"  to:    {message['to_member_id']}")
    lines.append(f"  type:  {message['type']}")
    if message.get("text"):
        lines.append(f"  text:  {message['text']}")
    return "\n".join(lines)


def format_indexed_list(
    items: list[Any],
    formatter: Callable[[Any], str],
    empty_msg: str,
) -> str:
    """Join formatted items with a single blank line between them.

    Items are not numbered — members reference messages by ``message_id``, not
    list index, so index markers would cost tokens without surfacing useful
    information.
    """
    if not items:
        return empty_msg
    return "\n\n".join(formatter(item) for item in items)


def format_member_detail(member: dict, *, full: bool = False) -> str:
    """Render a member (the ``member show`` shape) as text.

    ``full=False`` (default): 1-line compact ``<id> <name> <status>``.
    ``full=True``: labeled block exposing ``member_id``, ``name``, truncated
    ``description`` (60 codepoints + ``…``), ``status``, ``kind``, ``skills``
    (compact JSON array, ``-`` when empty), and the placement sub-block
    (``placement:   none`` when no placement row exists; ``None`` fields
    inside the placement render ``-``).
    """
    if not full:
        return f"{member['member_id']} {member['name']} {member['status']}"
    description = truncate_text(member["description"], full=False, limit=60)
    skills = member["skills"]
    lines = [
        f"  member_id:   {member['member_id']}",
        f"  name:        {member['name']}",
        f"  description: {description}",
        f"  status:      {member['status']}",
        f"  kind:        {member['kind']}",
        f"  skills:      {format_json(skills) if skills else '-'}",
    ]
    placement = member["placement"]
    if placement is None:
        lines.append("  placement:   none")
    else:
        lines.extend(
            [
                "  placement:",
                f"    backend:    {placement['coding_agent']}",
                f"    session:    {placement['mux_session']}",
                f"    window_id:  {placement['mux_window_id']}",
                f"    pane_id:    {_dash_if_none(placement['mux_pane_id'])}",
                f"    created_at: {placement['created_at']}",
            ]
        )
    return "\n".join(lines)


def _dash_if_none(value) -> str:
    return "-" if value is None else str(value)


def format_fleet_create(data: dict, *, full: bool = False) -> str:
    """Render the fleet-create result as text.

    ``full=False`` (default): 1-line compact form
    ``<fleet_id> director=<id>``.
    ``full=True``: 6-line block (fleet_id + director_member_id on
    their own lines, plus ``name``, ``created_at``, ``director_name``,
    ``pane``).
    """
    director = data["director"]
    if not full:
        return f"{data['fleet_id']} director={director['member_id']}"
    placement = director["placement"]
    lines = [
        str(data["fleet_id"]),
        str(director["member_id"]),
        f"name:             {data['name'] or ''}",
        f"created_at:       {data['created_at']}",
        f"director_name:    {director['name']}",
        f"pane:             {placement['mux_session']}:{placement['mux_window_id']}:{placement['mux_pane_id']}",
    ]
    return "\n".join(lines)


def format_member(data: dict, *, full: bool = False) -> str:
    """Render a member-create result as text.

    ``full=False`` (default): 1-line compact form
    ``<id> <name> backend=<coding_agent> pane=<pane_id>``.
    ``full=True``: 6-line block.
    """
    placement = data["placement"]
    if not full:
        pane = placement["mux_pane_id"] or "(pending)"
        return (
            f"{data['member_id']} {data['name']} "
            f"backend={placement['coding_agent']} pane={pane}"
        )
    lines = [
        "Member registered and spawned.",
        f"  member_id: {data['member_id']}",
        f"  name:      {data['name']}",
        f"  backend:   {placement['coding_agent']}",
        f"  pane_id:   {placement['mux_pane_id']}",
        f"  window_id: {placement['mux_window_id']}",
    ]
    return "\n".join(lines)


def _format_idle(seconds: int | None) -> str:
    if seconds is None:
        return "-"
    if seconds < 60:
        return f"{seconds}s"
    if seconds < 3600:
        return f"{seconds // 60}m"
    return f"{seconds // 3600}h"


def _format_ping_age(age_seconds: int | None) -> str:
    """Render a watched member's last-ping age for the status table."""
    if age_seconds is None:
        return "-"
    return f"{age_seconds}s ago"


def format_monitor_status(payload: dict) -> str:
    """Render ``monitor status`` as text: a runtime line + watched-member table.

    ``payload`` is ``{"runtime": {...}, "members": [...]}`` (the same shape the
    ``--json`` path emits). The runtime line reads ``running``/``stopped`` from
    the DB heartbeat; the table lists the watched set (the root Director + every
    ordinary member) with role / interval / last-ping age / enabled / pending /
    oldest-pending (``unacked``) age.
    The monitoring member is the unenrolled watcher and never appears here.
    """
    rt = payload["runtime"]
    if rt["running"]:
        line1 = (
            f"monitor: running (pid {rt['pid']}, "
            f"last tick {rt['last_tick_age_seconds']}s ago, "
            f"tick {rt['tick_seconds']}s, started {rt['started_at']})"
        )
    else:
        line1 = "monitor: stopped"
    lines = [line1]
    members = payload["members"]
    if members:
        lines.append(
            "  member_id  name         role      interval  last_ping  enabled  "
            "pending  unacked"
        )
        lines.append(
            "  ---------  -----------  --------  --------  ---------  -------  "
            "-------  -------"
        )
        for m in members:
            interval_s = f"{m['interval_seconds']}s"
            last_ping = _format_ping_age(m["last_ping_age_seconds"])
            enabled_s = "yes" if m["enabled"] else "no"
            unacked = _format_ping_age(m["oldest_pending_age_seconds"])
            lines.append(
                f"  {str(m['member_id']):<9}  {m['name']:<11}  {m['role']:<8}  "
                f"{interval_s:<8}  {last_ping:<9}  {enabled_s:<7}  "
                f"{str(m['pending_count']):<7}  {unacked}"
            )
    return "\n".join(lines)


def format_monitor_config(cfg: dict) -> str:
    """Render one member's ``monitor config`` row as a single compact line."""
    state = "enabled" if cfg["enabled"] else "disabled"
    last_ping = cfg["last_ping_at"] or "-"
    return (
        f"member {cfg['member_id']}: interval {cfg['interval_seconds']}s, "
        f"{state}, last_ping {last_ping}"
    )


def format_member_list(members: list) -> str:
    """Render the single ``member list`` table as text.

    One row per active registry entry of the fleet with ``member_id``,
    ``name``, ``kind``, ``backend``, ``pane_id``, and humanized ``idle``
    columns; a placementless row renders ``-`` in the ``backend`` and
    ``pane_id`` cells, and a placed row with no pane yet renders
    ``(pending)`` in ``pane_id``.
    """
    if not members:
        return "0 members."
    count = len(members)
    lines = [f"{count} member{'s' if count > 1 else ''}:"]
    lines.append("  member_id  name           kind      backend   pane_id  idle")
    lines.append("  ---------  -------------  --------  --------  -------  ----")
    for m in members:
        placement = m["placement"]
        if placement is None:
            backend = pane = "-"
        else:
            backend = placement["coding_agent"]
            pane = placement["mux_pane_id"] or "(pending)"
        lines.append(
            f"  {str(m['member_id']):<9}  {m['name']:<13}  {m['kind']:<8}  "
            f"{backend:<8}  {pane:<7}  {_format_idle(m['idle'])}"
        )
    return "\n".join(lines)
