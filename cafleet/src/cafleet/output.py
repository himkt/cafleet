import json
from typing import Any


def format_json(data: Any) -> str:
    return json.dumps(data, indent=2)


def format_register(data: dict) -> str:
    lines = [
        "Agent registered successfully!",
        f"  agent_id:  {data['agent_id']}",
        f"  name:      {data.get('name', '')}",
    ]
    return "\n".join(lines)


def format_task(task: dict) -> str:
    if "task" in task:
        task = task["task"]
    task_id = task.get("id", "?")
    state = task.get("status", {}).get("state", "?")
    from_agent = task.get("metadata", {}).get("fromAgentId", "?")
    to_agent = task.get("metadata", {}).get("toAgentId", "?")
    msg_type = task.get("metadata", {}).get("type", "?")
    text = ""
    for artifact in task.get("artifacts", []):
        for part in artifact.get("parts", []):
            if isinstance(part, dict) and part.get("text"):
                text = part["text"]
                break
        if text:
            break
    lines = [
        f"  id:    {task_id}",
        f"  state: {state}",
        f"  from:  {from_agent}",
        f"  to:    {to_agent}",
        f"  type:  {msg_type}",
    ]
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
        f"  agent_id:    {agent.get('agent_id', '?')}",
        f"  name:        {agent.get('name', '?')}",
        f"  description: {agent.get('description', '?')}",
        f"  status:      {agent.get('status', 'active')}",
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
    """Human-friendly ``session create`` text (design 0000026).

    Line 1 is the ``session_id`` (preserves backward-compatible script usage
    that parses only the first line). Line 2 is the root Director's ``agent_id``.
    The remaining lines are key-value pairs.
    """
    director = data.get("director", {}) or {}
    placement = director.get("placement", {}) or {}
    pane = (
        f"{placement.get('tmux_session', '?')}:"
        f"{placement.get('tmux_window_id', '?')}:"
        f"{placement.get('tmux_pane_id', '?')}"
    )
    lines = [
        data.get("session_id", "?"),
        director.get("agent_id", "?"),
        f"label:            {data.get('label') or ''}",
        f"created_at:       {data.get('created_at', '')}",
        f"director_name:    {director.get('name', '')}",
        f"pane:             {pane}",
        f"administrator:    {data.get('administrator_agent_id', '')}",
    ]
    return "\n".join(lines)


def format_member(data: dict) -> str:
    placement = data.get("placement", {}) or {}
    lines = [
        "Member registered and spawned.",
        f"  agent_id:  {data.get('agent_id', '?')}",
        f"  name:      {data.get('name', '?')}",
        f"  backend:   {placement.get('coding_agent', 'claude')}",
        f"  pane_id:   {placement.get('tmux_pane_id', '?')}",
        f"  window_id: {placement.get('tmux_window_id', '?')}",
    ]
    return "\n".join(lines)


def format_member_list(members: list) -> str:
    count = len(members)
    if count == 0:
        return "0 members."
    lines = [f"{count} member{'s' if count != 1 else ''}:"]
    header = "  agent_id        name      status  backend  session  window_id  pane_id  created_at"
    sep = (
        "  --------------  --------  ------  -------  -------  ---------  -------  "
        "--------------------"
    )
    lines.append(header)
    lines.append(sep)
    for m in members:
        placement = m.get("placement", {}) or {}
        pane_id = placement.get("tmux_pane_id")
        pane_display = pane_id if pane_id is not None else "(pending)"
        agent_id = m.get("agent_id", "?")
        if len(agent_id) > 14:
            agent_id = agent_id[:12] + "…"
        lines.append(
            f"  {agent_id:<14}  {m.get('name', '?'):<8}  "
            f"{m.get('status', 'active'):<6}  "
            f"{placement.get('coding_agent', 'claude'):<7}  "
            f"{placement.get('tmux_session', '?'):<7}  "
            f"{placement.get('tmux_window_id', '?'):<9}  "
            f"{pane_display:<7}  "
            f"{placement.get('created_at', m.get('registered_at', '?'))}"
        )
    return "\n".join(lines)
