"""Human-readable text formatters."""

from collections.abc import Callable
from typing import Any

from cafleet.output.render import format_json, render_task, truncate_text


def format_task(task: dict, *, full: bool = False) -> str:
    """Render a task as text.

    ``full=False`` (default): 2-line compact render —
    line 1 is ``[<id> | from:<from> | <ts>]`` (with optional
    ``| kind:<kind>`` and ``| origin:<id>`` segments), line 2 is the body.

    ``full=True``: 6-line verbose layout that exposes every typed column
    (``id``, ``state``, ``from``, ``to``, ``type``, ``text``).
    """
    if "task" in task and isinstance(task["task"], dict):
        task = task["task"]
    if not full:
        rendered = render_task(task, full=False)
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
        f"  id:    {task['task_id']}",
        f"  state: {task['status_state']}",
        f"  from:  {task['from_agent_id']}",
    ]
    if task.get("to_agent_id") is not None:
        lines.append(f"  to:    {task['to_agent_id']}")
    lines.append(f"  type:  {task['type']}")
    if task.get("text"):
        lines.append(f"  text:  {task['text']}")
    return "\n".join(lines)


def format_indexed_list(
    items: list[Any],
    formatter: Callable[[Any], str],
    empty_msg: str,
) -> str:
    """Join formatted items with a single blank line between them.

    Items are not numbered — agents reference tasks by ``task_id``, not list
    index, so index markers would cost tokens without surfacing useful
    information.
    """
    if not items:
        return empty_msg
    return "\n\n".join(formatter(item) for item in items)


def format_agent(agent: dict, *, full: bool = False) -> str:
    """Render an agent (the ``member show`` shape) as text.

    ``full=False`` (default): 1-line compact ``<id> <name> <status>``.
    ``full=True``: labeled block exposing ``agent_id``, ``name``, truncated
    ``description`` (60 codepoints + ``…``), ``status``, ``kind``, ``skills``
    (compact JSON array, ``-`` when empty), and the placement sub-block
    (``placement:   none`` when no placement row exists; ``None`` fields
    inside the placement render ``-``).
    """
    if not full:
        return f"{agent['agent_id']} {agent['name']} {agent['status']}"
    description = truncate_text(agent["description"], full=False, limit=60)
    skills = agent["skills"]
    lines = [
        f"  agent_id:    {agent['agent_id']}",
        f"  name:        {agent['name']}",
        f"  description: {description}",
        f"  status:      {agent['status']}",
        f"  kind:        {agent['kind']}",
        f"  skills:      {format_json(skills) if skills else '-'}",
    ]
    placement = agent["placement"]
    if placement is None:
        lines.append("  placement:   none")
    else:
        lines.extend(
            [
                "  placement:",
                f"    director_agent_id: {_dash_if_none(placement['director_agent_id'])}",
                f"    backend:           {placement['coding_agent']}",
                f"    session:           {placement['tmux_session']}",
                f"    window_id:         {placement['tmux_window_id']}",
                f"    pane_id:           {_dash_if_none(placement['tmux_pane_id'])}",
                f"    created_at:        {placement['created_at']}",
            ]
        )
    return "\n".join(lines)


def _dash_if_none(value) -> str:
    return "-" if value is None else str(value)


def format_fleet_create(data: dict, *, full: bool = False) -> str:
    """Render the fleet-create result as text.

    ``full=False`` (default): 1-line compact form
    ``<fleet_id> director=<id> admin=<id>``.
    ``full=True``: 7-line block (fleet_id + director_agent_id on
    their own lines, plus ``label``, ``created_at``, ``director_name``,
    ``pane``, ``administrator``).
    """
    director = data["director"]
    if not full:
        return (
            f"{data['fleet_id']} "
            f"director={director['agent_id']} "
            f"admin={data['administrator_agent_id']}"
        )
    placement = director["placement"]
    lines = [
        str(data["fleet_id"]),
        str(director["agent_id"]),
        f"label:            {data['label'] or ''}",
        f"created_at:       {data['created_at']}",
        f"director_name:    {director['name']}",
        f"pane:             {placement['tmux_session']}:{placement['tmux_window_id']}:{placement['tmux_pane_id']}",
        f"administrator:    {data['administrator_agent_id']}",
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
        pane = placement["tmux_pane_id"] or "(pending)"
        return (
            f"{data['agent_id']} {data['name']} "
            f"backend={placement['coding_agent']} pane={pane}"
        )
    lines = [
        "Member registered and spawned.",
        f"  agent_id:  {data['agent_id']}",
        f"  name:      {data['name']}",
        f"  backend:   {placement['coding_agent']}",
        f"  pane_id:   {placement['tmux_pane_id']}",
        f"  window_id: {placement['tmux_window_id']}",
    ]
    return "\n".join(lines)


_AGENT_ID_COLUMN_WIDTH = 14


def _agent_id_for_column(agent_id: int) -> str:
    return str(agent_id)


def format_member_list_activity(members: list) -> str:
    """Render the activity-augmented member roster as text.

    One row per member with the four activity proxies —
    ``last_sent`` / ``last_recv`` / ``last_ack`` / ``idle``. Timestamps are
    rendered as ``HH:MM:SS`` (the time portion of the ISO string);
    ``idle`` is humanized to ``Ns`` / ``Nm`` / ``Nh``.
    """
    if not members:
        return "0 members."
    count = len(members)
    lines = [f"{count} member{'s' if count > 1 else ''}:"]
    header = "  agent_id        name      status  last_sent  last_recv  last_ack   idle"
    sep = "  --------------  --------  ------  ---------  ---------  ---------  -----"
    lines.append(header)
    lines.append(sep)
    for m in members:
        agent_id = _agent_id_for_column(m["agent_id"])
        lines.append(
            f"  {agent_id:<{_AGENT_ID_COLUMN_WIDTH}}  {m['name']:<8}  "
            f"{m['status']:<6}  "
            f"{_format_iso_hms(m['last_sent']):<9}  "
            f"{_format_iso_hms(m['last_recv']):<9}  "
            f"{_format_iso_hms(m['last_ack']):<9}  "
            f"{_format_idle(m['idle'])}"
        )
    return "\n".join(lines)


def _format_iso_hms(iso_ts: str | None) -> str:
    if iso_ts is None:
        return "-"
    try:
        return iso_ts.split("T")[1][:8]
    except (IndexError, AttributeError):
        return "-"


def _format_idle(seconds: int | None) -> str:
    if seconds is None:
        return "-"
    if seconds < 60:
        return f"{seconds}s"
    if seconds < 3600:
        return f"{seconds // 60}m"
    return f"{seconds // 3600}h"


def _format_ping_age(age_seconds: int | None) -> str:
    """Render a watched agent's last-ping age for the status table."""
    if age_seconds is None:
        return "—"
    return f"{age_seconds}s ago"


def format_monitor_status(payload: dict) -> str:
    """Render ``monitor status`` as text: a runtime line + watched-agent table.

    ``payload`` is ``{"runtime": {...}, "agents": [...]}`` (the same shape the
    ``--json`` path emits). The runtime line reads ``running``/``stopped`` from
    the DB heartbeat; the table lists the watched set (the root Director + every
    ordinary member) with role / interval / last-ping age / enabled / pending.
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
    agents = payload["agents"]
    if agents:
        lines.append(
            "  agent_id  name         role      interval  last_ping  enabled  pending"
        )
        lines.append(
            "  --------  -----------  --------  --------  ---------  -------  -------"
        )
        for a in agents:
            interval_s = f"{a['interval_seconds']}s"
            last_ping = _format_ping_age(a["last_ping_age_seconds"])
            enabled_s = "yes" if a["enabled"] else "no"
            lines.append(
                f"  {str(a['agent_id']):<8}  {a['name']:<11}  {a['role']:<8}  "
                f"{interval_s:<8}  {last_ping:<9}  {enabled_s:<7}  "
                f"{a['pending_count']}"
            )
    return "\n".join(lines)


def format_monitor_config(cfg: dict) -> str:
    """Render one agent's ``monitor config`` row as a single compact line."""
    state = "enabled" if cfg["enabled"] else "disabled"
    last_ping = cfg["last_ping_at"] or "-"
    return (
        f"agent {cfg['agent_id']}: interval {cfg['interval_seconds']}s, "
        f"{state}, last_ping {last_ping}"
    )


def format_member_roster(agents: list) -> str:
    """Render the ``member list --all`` roster as text.

    One row per active agent of the fleet with a ``kind`` column; every
    placement cell renders ``-`` for a placementless row, and a placed row
    with no pane yet renders ``(pending)`` in ``pane_id`` (matching
    :func:`format_member_list`).
    """
    if not agents:
        return "0 agents."
    count = len(agents)
    lines = [f"{count} agent{'s' if count > 1 else ''}:"]
    lines.append(
        "  agent_id  name           status  kind           backend  session  "
        "window_id  pane_id  created_at"
    )
    lines.append(
        "  --------  -------------  ------  -------------  -------  -------  "
        "---------  -------  --------------------"
    )
    for a in agents:
        placement = a["placement"]
        if placement is None:
            backend = session_name = window_id = pane = created_at = "-"
        else:
            backend = placement["coding_agent"]
            session_name = placement["tmux_session"]
            window_id = placement["tmux_window_id"]
            pane = placement["tmux_pane_id"] or "(pending)"
            created_at = placement["created_at"]
        lines.append(
            f"  {str(a['agent_id']):<8}  {a['name']:<13}  {a['status']:<6}  "
            f"{a['kind']:<13}  {backend:<7}  {session_name:<7}  "
            f"{window_id:<9}  {pane:<7}  {created_at}"
        )
    return "\n".join(lines)


def format_member_list(members: list) -> str:
    if not members:
        return "0 members."
    count = len(members)
    lines = [f"{count} member{'s' if count > 1 else ''}:"]
    header = "  agent_id        name      status  backend  session  window_id  pane_id  created_at"
    sep = (
        "  --------------  --------  ------  -------  -------  ---------  -------  "
        "--------------------"
    )
    lines.append(header)
    lines.append(sep)
    for m in members:
        placement = m["placement"]
        pane_display = placement["tmux_pane_id"] or "(pending)"
        agent_id = _agent_id_for_column(m["agent_id"])
        lines.append(
            f"  {agent_id:<{_AGENT_ID_COLUMN_WIDTH}}  {m['name']:<8}  "
            f"{m['status']:<6}  "
            f"{placement['coding_agent']:<7}  "
            f"{placement['tmux_session']:<7}  "
            f"{placement['tmux_window_id']:<9}  "
            f"{pane_display:<7}  "
            f"{placement['created_at']}"
        )
    return "\n".join(lines)
