import json
from typing import Any


def format_json(data: Any) -> str:
    return json.dumps(data, indent=2)


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
            for artifact in task.get("artifacts", [])
            for part in artifact.get("parts", [])
            if isinstance(part, dict) and part.get("text")
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


def format_task_list(tasks: list) -> str:
    if not tasks:
        return "No messages found."
    parts = []
    for i, task in enumerate(tasks):
        parts.append(f"[{i + 1}]")
        parts.append(format_task(task))
    return "\n".join(parts)


def format_agent(agent: dict) -> str:
    lines = [
        f"  agent_id:    {agent['agent_id']}",
        f"  name:        {agent['name']}",
        f"  description: {agent['description']}",
        f"  status:      {agent['status']}",
    ]
    return "\n".join(lines)


def format_agent_list(agents: list) -> str:
    if not agents:
        return "No agents found."
    parts = []
    for i, agent in enumerate(agents):
        parts.append(f"[{i + 1}]")
        parts.append(format_agent(agent))
    return "\n".join(parts)


def format_session_create(data: dict) -> str:
    """Render the ``session create`` text block.

    Line 1 is the session_id so script consumers that only parse the first
    line keep working; line 2 is the root Director's agent_id.
    """
    director = data["director"]
    placement = director["placement"]
    pane = (
        f"{placement['tmux_session']}:"
        f"{placement['tmux_window_id']}:"
        f"{placement['tmux_pane_id']}"
    )
    lines = [
        data["session_id"],
        director["agent_id"],
        f"label:            {data['label'] or ''}",
        f"created_at:       {data['created_at']}",
        f"director_name:    {director['name']}",
        f"pane:             {pane}",
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
        pane_id = placement["tmux_pane_id"]
        pane_display = pane_id if pane_id is not None else "(pending)"
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
