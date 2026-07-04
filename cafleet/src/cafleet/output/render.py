"""Wire-shape projections, truncation, JSON rendering, and ANSI stripping."""

import json
import re
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


def truncate_task_text(result: Any, *, full: bool) -> Any:
    if full:
        return result
    items = result if isinstance(result, list) else [result]
    for item in items:
        task = item.get("task", item) if isinstance(item, dict) else None
        if not isinstance(task, dict):
            continue
        if "text" in task:
            task["text"] = truncate_text(task["text"], full=full)
    return result


def render_task(task: dict, *, full: bool = False) -> dict:
    """Project a typed-column task dict to the compact rendered shape.

    ``full=True`` returns the typed-column dict unchanged (no projection).
    ``full=False`` (default) returns a new dict with ``id``, ``from``,
    ``ts``, ``text``, plus optional ``kind`` (when ``type`` ≠ ``"unicast"``)
    and ``origin`` (only when ``origin_task_id`` is non-NULL).
    """
    if full:
        return task
    out: dict = {
        "id": task["task_id"],
        "from": task["from_agent_id"],
        "ts": task["status_timestamp"],
        "text": task["text"],
    }
    if task["type"] != "unicast":
        out["kind"] = task["type"]
    if task.get("origin_task_id"):
        out["origin"] = task["origin_task_id"]
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
