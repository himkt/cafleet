import json
import re
from collections.abc import Callable
from typing import Any

from cafleet.config import settings

_TRUNCATION_SUFFIX = "…"

_ANSI_ESCAPE_RE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


def strip_ansi(text: str) -> str:
    """Strip ANSI CSI escape sequences and collapse \\r-rewritten line segments.

    Carriage-return de-fragmentation: TUI redraws emit ``...prefix\\rNEW``
    sequences where ``NEW`` overwrites ``prefix`` on the same line. We keep
    only the segment after the last ``\\r`` per line so the captured buffer
    matches what an operator sees.
    """
    if not text:
        return text
    cleaned = _ANSI_ESCAPE_RE.sub("", text)
    return "\n".join(line.rsplit("\r", 1)[-1] for line in cleaned.split("\n"))


def format_json(data: Any) -> str:
    """Render ``data`` as compact JSON.

    Compact (no whitespace separators) so per-poll envelopes stay short for
    agent consumers. ``ensure_ascii=False`` keeps non-ASCII (e.g. the ``…``
    truncation suffix) as UTF-8 rather than ``\\uXXXX`` escapes, matching the
    UTF-8 byte budgets.
    """
    return json.dumps(data, separators=(",", ":"), ensure_ascii=False)


def truncate_text(
    value: str | None, *, full: bool, limit: int | None = None
) -> str | None:
    """Truncate ``value`` to ``limit`` codepoints + the ``…`` suffix.

    When ``limit`` is ``None`` the helper falls back to
    ``settings.max_text_len`` (env var ``CAFLEET_MAX_TEXT_LEN``, default
    ``200``). ``full=True`` returns ``value`` unchanged.
    """
    if full or value is None:
        return value
    effective_limit = limit if limit is not None else settings.max_text_len
    if len(value) <= effective_limit:
        return value
    return value[:effective_limit] + _TRUNCATION_SUFFIX


def truncate_task_text(result: Any, *, full: bool, limit: int | None = None) -> Any:
    if full:
        return result
    items = result if isinstance(result, list) else [result]
    for item in items:
        task = item.get("task", item) if isinstance(item, dict) else None
        if not isinstance(task, dict):
            continue
        if "text" in task:
            task["text"] = truncate_text(task["text"], full=full, limit=limit)
    return result


def render_task(task: dict, *, full: bool = False) -> dict:
    """Project a typed-column task dict to the compact rendered shape.

    ``full=True`` returns the typed-column dict unchanged (no projection,
    no UUID prefix-rendering). ``full=False`` (default) returns a new dict
    with ``id`` (8-char prefix), ``from`` (8-char prefix), ``ts``, ``text``,
    plus optional ``kind`` (when ``type`` ≠ ``"unicast"``) and ``origin``
    (8-char prefix; only when ``origin_task_id`` is non-NULL).
    """
    if full:
        return task
    out: dict = {
        "id": task["task_id"][:8],
        "from": task["from_agent_id"][:8],
        "ts": task["status_timestamp"],
        "text": task["text"],
    }
    if task["type"] != "unicast":
        out["kind"] = task["type"]
    if task.get("origin_task_id"):
        out["origin"] = task["origin_task_id"][:8]
    return out


def render_tasks_in_result(result: Any, *, full: bool) -> Any:
    """Apply ``render_task`` to every task dict in a broker result structure.

    ``full=True`` returns ``result`` unchanged. Otherwise walks lists,
    ``{"task": ...}`` envelopes, and bare flat task dicts; returns a new
    structure (does not mutate ``result``).
    """
    if full:
        return result
    if isinstance(result, list):
        return [_render_item(item) for item in result]
    return _render_item(result)


def _render_item(item: Any) -> Any:
    if not isinstance(item, dict):
        return item
    if "task" in item and isinstance(item["task"], dict) and "task_id" in item["task"]:
        new = dict(item)
        new["task"] = render_task(item["task"], full=False)
        return new
    if "task_id" in item:
        return render_task(item, full=False)
    return item


def render_agent(agent: dict, *, full: bool = False) -> dict:
    """Project a broker agent dict to the slim wire shape.

    Slim shape (default): ``id`` (8-char prefix of ``agent_id``), ``name``,
    ``description`` (truncated to 60 codepoints + ``…`` suffix), ``status``,
    plus ``coding_agent`` when the source dict carries a ``placement`` with
    that key. ``full=True`` returns ``agent`` unchanged.
    """
    if full:
        return agent
    out: dict = {
        "id": agent["agent_id"][:8],
        "name": agent["name"],
        "description": truncate_text(agent["description"], full=False, limit=60),
        "status": agent["status"],
    }
    placement = agent.get("placement")
    if placement and "coding_agent" in placement:
        out["coding_agent"] = placement["coding_agent"]
    return out


def render_agents_in_result(result: Any, *, full: bool) -> Any:
    """Apply ``render_agent`` to every agent dict in a broker result structure.

    ``full=True`` returns ``result`` unchanged. Otherwise walks lists and
    bare flat agent dicts; returns a new structure (does not mutate ``result``).
    """
    if full:
        return result
    if isinstance(result, list):
        return [_render_agent_item(item) for item in result]
    return _render_agent_item(result)


def _render_agent_item(item: Any) -> Any:
    if isinstance(item, dict) and "agent_id" in item:
        return render_agent(item, full=False)
    return item


def format_register(data: dict) -> str:
    lines = [
        "Agent registered successfully!",
        f"  agent_id:  {data['agent_id']}",
        f"  name:      {data['name']}",
    ]
    return "\n".join(lines)


def format_task(task: dict, *, full: bool = False) -> str:
    """Render a task as text.

    ``full=False`` (default): 2-line compact render —
    line 1 is ``[<id8> | from:<from8> | <ts>]`` (with optional
    ``| kind:<kind>`` and ``| origin:<id8>`` segments), line 2 is the body.

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
    if task.get("to_agent_id"):
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

    Items are not numbered — agents reference tasks by ``task_id`` (8-char
    prefix), not list index, so index markers would cost tokens without
    surfacing useful information.
    """
    if not items:
        return empty_msg
    return "\n\n".join(formatter(item) for item in items)


def format_agent(agent: dict, *, full: bool = False) -> str:
    """Render an agent as text.

    ``full=False`` (default): 1-line compact ``<id8> <name> <status>``.
    ``full=True``: 4-line block exposing the full ``agent_id``, ``name``,
    truncated ``description`` (60 codepoints + ``…``), and ``status``.
    """
    if not full:
        return f"{agent['agent_id'][:8]} {agent['name']} {agent['status']}"
    description = truncate_text(agent["description"], full=False, limit=60)
    lines = [
        f"  agent_id:    {agent['agent_id']}",
        f"  name:        {agent['name']}",
        f"  description: {description}",
        f"  status:      {agent['status']}",
    ]
    return "\n".join(lines)


def format_session_create(data: dict, *, full: bool = False) -> str:
    """Render the session-create result as text.

    ``full=False`` (default): 1-line compact form
    ``<session_id> director=<id8> admin=<id8>``.
    ``full=True``: 7-line block (session_id + director_agent_id on
    their own lines, plus ``label``, ``created_at``, ``director_name``,
    ``pane``, ``administrator``).
    """
    director = data["director"]
    if not full:
        return (
            f"{data['session_id']} "
            f"director={director['agent_id'][:8]} "
            f"admin={data['administrator_agent_id'][:8]}"
        )
    placement = director["placement"]
    lines = [
        data["session_id"],
        director["agent_id"],
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
    ``<id8> <name> backend=<coding_agent> pane=<pane_id>``.
    ``full=True``: 6-line block.
    """
    placement = data["placement"]
    if not full:
        pane = placement["tmux_pane_id"] or "(pending)"
        return (
            f"{data['agent_id'][:8]} {data['name']} "
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


def _agent_id_for_column(agent_id: str) -> str:
    if len(agent_id) > _AGENT_ID_COLUMN_WIDTH:
        return agent_id[: _AGENT_ID_COLUMN_WIDTH - 2] + "…"
    return agent_id


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
