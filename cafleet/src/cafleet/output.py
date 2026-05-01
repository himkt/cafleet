import json
from collections.abc import Callable
from typing import Any


def format_json(data: Any) -> str:
    return json.dumps(data, indent=2)


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
        for artifact in task.get("artifacts", []) or []:
            for part in artifact.get("parts", []) or []:
                if "text" in part:
                    part["text"] = truncate_text(part["text"], full=full, limit=limit)
    return result


def format_register(data: dict) -> str:
    lines = [
        "Agent registered successfully!",
        f"  agent_id:  {data['agent_id']}",
        f"  name:      {data['name']}",
    ]
    return "\n".join(lines)


def format_task(task: dict) -> str:
    if "task" in task:
        task = task["task"]
    metadata = task["metadata"]
    text = next(
        (
            part["text"]
            for artifact in task["artifacts"]
            for part in artifact["parts"]
            if part.get("text")
        ),
        "",
    )
    lines = [
        f"  id:    {task['id']}",
        f"  state: {task['status']['state']}",
        f"  from:  {metadata['fromAgentId']}",
    ]
    # ``toAgentId`` is absent on broadcast_summary tasks; elide the line
    # rather than rendering a "?" placeholder that hides the distinction.
    if "toAgentId" in metadata:
        lines.append(f"  to:    {metadata['toAgentId']}")
    lines.append(f"  type:  {metadata['type']}")
    if text:
        lines.append(f"  text:  {text}")
    return "\n".join(lines)


def format_indexed_list(
    items: list[Any],
    formatter: Callable[[Any], str],
    empty_msg: str,
) -> str:
    if not items:
        return empty_msg
    parts = []
    for i, item in enumerate(items, start=1):
        parts.append(f"[{i}]")
        parts.append(formatter(item))
    return "\n".join(parts)


def format_agent(agent: dict) -> str:
    lines = [
        f"  agent_id:    {agent['agent_id']}",
        f"  name:        {agent['name']}",
        f"  description: {agent['description']}",
        f"  status:      {agent['status']}",
    ]
    return "\n".join(lines)


def format_session_create(data: dict) -> str:
    director = data["director"]
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


def format_member(data: dict) -> str:
    placement = data["placement"]
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
