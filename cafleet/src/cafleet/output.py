import json
from collections.abc import Callable
from typing import Any


def format_json(data: Any, *, pretty: bool = False) -> str:
    """Render ``data`` as JSON.

    Default is compact (no whitespace separators) so per-poll envelopes stay
    short for agent consumers. Pass ``pretty=True`` for ``indent=2`` output
    when a human is reading the result.
    """
    if pretty:
        return json.dumps(data, indent=2)
    return json.dumps(data, separators=(",", ":"))


def truncate_text(value: str | None, *, full: bool, limit: int = 10) -> str | None:
    if full or value is None or len(value) <= limit:
        return value
    return value[:limit] + "..."


def truncate_task_text(result: Any, *, full: bool, limit: int = 10) -> Any:
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

    ``full=True``: 6-line legacy verbose layout that exposes every typed
    column (``id``, ``state``, ``from``, ``to``, ``type``, ``text``).
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

    Surface 17 dropped the legacy ``[1]`` / ``[2]`` index markers — agents
    reference tasks by ``task_id`` (8-char prefix), not list index, so the
    markers cost tokens without surfacing useful information.
    """
    if not items:
        return empty_msg
    return "\n\n".join(formatter(item) for item in items)


def format_agent(agent: dict, *, full: bool = False) -> str:
    """Render an agent as text.

    ``full=False`` (default): 1-line compact ``<id8> <name> <status>``.
    ``full=True``: 4-line legacy block exposing the full ``agent_id``,
    ``name``, ``description``, and ``status``.
    """
    if not full:
        return f"{agent['agent_id'][:8]} {agent['name']} {agent['status']}"
    lines = [
        f"  agent_id:    {agent['agent_id']}",
        f"  name:        {agent['name']}",
        f"  description: {agent['description']}",
        f"  status:      {agent['status']}",
    ]
    return "\n".join(lines)


def format_session_create(data: dict, *, full: bool = False) -> str:
    """Render the session-create result as text.

    ``full=False`` (default): 1-line compact form
    ``<session_id> director=<id8> admin=<id8>``.
    ``full=True``: 7-line legacy block (session_id + director_agent_id on
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
    ``full=True``: 6-line legacy block.
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
        agent_id = m["agent_id"]
        if len(agent_id) > _AGENT_ID_COLUMN_WIDTH:
            agent_id = agent_id[: _AGENT_ID_COLUMN_WIDTH - 2] + "…"
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
