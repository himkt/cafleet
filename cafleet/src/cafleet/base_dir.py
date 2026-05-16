"""Base directory resolver for CAFleet (design 0000055; task-scope branch design 0000060).

Owns the deterministic side of `${BASE}` resolution: anchor file
read / write / validate, the no-positional resolution branches
(`resolved` / `needs-user-input`), the positional `task_name` task-scope
branch (`resolved` with `source="task-scope"`, or `unset` when an absolute
path lives outside any recognized task folder), the fatal `AnchorError`
raise on schema / version mismatch, and the `<unset>` sentinel.

The `AskUserQuestion` branch lives in Claude's tool context — this module
only reports `status="needs-user-input"` with the candidates Claude should
present; it does not prompt the user itself.
"""

from __future__ import annotations

import json
import os
import re
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

UNSET_SENTINEL = "<unset>"
ANCHOR_FILENAME = ".cafleet-base-dir.json"
ANCHOR_VERSION = 1

_TMP_CANDIDATE = Path("/tmp/claude-code")

_BUCKET_SLUG_RE = {
    "design-docs": re.compile(r"^\d{7}-[A-Za-z0-9_-]+$"),
    "researches": re.compile(r"^[A-Za-z0-9][A-Za-z0-9_-]*$"),
}

_BASE_INSERT_MARKER = (
    "[INSERT abs BASE path the Director resolved via Skill(cafleet:base-dir)]"
)
_BASE_LINE_MARKER = f"BASE: {_BASE_INSERT_MARKER}"


class AnchorError(Exception):
    """An on-disk anchor is malformed, mismatched, or version-incompatible."""


def _now_iso8601() -> str:
    return datetime.now(UTC).isoformat(timespec="microseconds")


def _is_under(path: Path, parent: Path) -> bool:
    try:
        return path.is_relative_to(parent)
    except ValueError:
        return False


def _validate_anchor(anchor_path: Path, data: Any) -> None:
    if not isinstance(data, dict):
        raise AnchorError(f"anchor at {anchor_path} is not a JSON object")
    version = data.get("version")
    if not isinstance(version, int) or version != ANCHOR_VERSION:
        raise AnchorError(
            f"anchor at {anchor_path} has version={version}; "
            f"this cafleet supports version {ANCHOR_VERSION}. "
            "Delete the anchor and re-resolve."
        )
    base_field = data.get("base")
    if not isinstance(base_field, str) or not base_field:
        raise AnchorError(f"anchor at {anchor_path} is missing the 'base' field")
    if not Path(base_field).is_absolute():
        raise AnchorError(
            f"anchor at {anchor_path} has non-absolute base={base_field!r}; "
            "the anchor contract requires an absolute path. "
            "Delete the anchor and re-resolve."
        )
    source_field = data.get("source")
    if source_field not in ("cwd-inference", "askuserquestion", "task-scope"):
        raise AnchorError(
            f"anchor at {anchor_path} has invalid source={source_field!r}; "
            "must be 'cwd-inference', 'askuserquestion', or 'task-scope'. "
            "Delete the anchor and re-resolve."
        )


def _load_anchor(anchor_path: Path) -> dict[str, Any]:
    try:
        data = json.loads(anchor_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise AnchorError(f"anchor at {anchor_path} is not valid JSON: {exc}") from exc
    _validate_anchor(anchor_path, data)
    return data


def _read_consistent_anchor(anchor_path: Path) -> str | None:
    if not anchor_path.is_file():
        return None
    data = _load_anchor(anchor_path)
    base_field = data["base"]
    # Compare via Path.resolve() so equivalent paths with different lexical
    # forms (symlinks, `..` segments, trailing slash) do not false-positive
    # a fatal mismatch. strict=False keeps non-existent components tolerated
    # (e.g., the fixture-based tests that write `base` pointing at a sibling
    # tmp dir which may not exist on every platform).
    base_resolved = Path(base_field).resolve(strict=False)
    parent_resolved = anchor_path.parent.resolve(strict=False)
    if base_resolved != parent_resolved:
        raise AnchorError(
            f"anchor file {anchor_path} records base={base_field} "
            f"but lives at {anchor_path.parent}; refusing to use."
        )
    return base_field


def _write_anchor(anchor_path: Path, *, base: str, source: str) -> None:
    payload = {
        "version": ANCHOR_VERSION,
        "base": base,
        "source": source,
        "resolved_at": _now_iso8601(),
    }
    anchor_path.parent.mkdir(parents=True, exist_ok=True)
    anchor_path.write_text(json.dumps(payload), encoding="utf-8")


def _infer_repo_root(cwd: Path) -> Path | None:
    for candidate in (cwd, *cwd.parents):
        if is_git_repo_root(candidate):
            return candidate
    return None


def _match_known_task_pattern(path: Path, repo_root: Path) -> Path | None:
    for candidate in (path, *path.parents):
        parent = candidate.parent
        if parent == candidate or parent.parent != repo_root:
            continue
        slug_re = _BUCKET_SLUG_RE.get(parent.name)
        if slug_re is None or not slug_re.match(candidate.name):
            continue
        if candidate.exists() and not candidate.is_dir():
            continue
        return candidate
    return None


_UNSET_SHAPE = {
    "status": "unset",
    "base": None,
    "source": "absolute-path-arg",
    "anchor": None,
}


def _resolve_task_scope(task_name: str, *, cwd: Path) -> dict[str, Any]:
    repo_root = _infer_repo_root(cwd)
    if repo_root is None:
        raise RuntimeError(
            f"cannot resolve task-scope base-dir: no .git ancestor "
            f"found from CWD {cwd}. cd to the repo root and retry."
        )

    candidate = Path(task_name)
    if candidate.is_absolute():
        task_folder = _match_known_task_pattern(candidate, repo_root)
        if task_folder is None:
            return {**_UNSET_SHAPE, "task_name": task_name}
    else:
        joined = (repo_root / candidate).resolve(strict=False)
        if joined != repo_root and not _is_under(joined, repo_root):
            raise RuntimeError(
                f"task_name {task_name!r} resolves outside the repo root "
                f"({joined} is not under {repo_root}); refusing to create "
                f"a task folder outside the repo."
            )
        task_folder = _match_known_task_pattern(joined, repo_root) or joined

    task_folder.mkdir(parents=True, exist_ok=True)
    anchor_path = task_folder / ANCHOR_FILENAME

    if _read_consistent_anchor(anchor_path) is not None:
        source = "anchor"
    else:
        _write_anchor(anchor_path, base=str(task_folder), source="task-scope")
        source = "task-scope"

    return {
        "status": "resolved",
        "base": str(task_folder),
        "source": source,
        "anchor": str(anchor_path),
        "task_name": task_name,
    }


def resolve(
    *,
    task_name: str | None = None,
    cwd: Path | None = None,
    home: Path | None = None,
    tmp_candidate: Path | None = None,
) -> dict[str, Any]:
    """Resolve the `${BASE}` output-root.

    Returns one of three shapes (matches the contract in
    `skills/base-dir/SKILL.md`):

    - ``{"status": "resolved", "base": "<abs>", "source": "cwd-inference" | "anchor"
        | "task-scope", "anchor": "<abs>/.cafleet-base-dir.json", ...}``
    - ``{"status": "unset", "base": None, "source": "absolute-path-arg",
        "anchor": None, "task_name": <task-name>}`` (only emitted by the
        positional ``task_name`` branch when an absolute path lives outside
        any recognized task-folder pattern)
    - ``{"status": "needs-user-input", "base": None, "source": None,
        "candidates": ["<tmp-candidate>", "<cwd>"]}``

    When ``task_name`` is supplied, the function dispatches to the
    task-scope branch (per design 0000060 §Spec 2): infers the repo root
    from ``cwd``, joins relative paths under it (or matches absolute paths
    against the known ``researches/<slug>/`` and ``design-docs/<slug>/``
    patterns), auto-creates the task folder, and writes the anchor inline.
    The result includes a ``task_name`` field echoing the input. The
    no-positional path is unchanged.

    Raises ``AnchorError`` on schema / version mismatch or anchor
    inconsistency (the fatal branch — the CLI surfaces this as a non-zero
    exit instead of one of the JSON shapes above). Raises ``RuntimeError``
    when ``task_name`` is supplied and ``cwd`` has no ``.git`` ancestor.
    """
    cwd_path = Path(cwd) if cwd is not None else Path.cwd()
    if task_name is not None:
        return _resolve_task_scope(task_name, cwd=cwd_path)

    if home is not None:
        home_path = Path(home)
    else:
        home_env = os.environ.get("HOME")
        if not home_env:
            raise RuntimeError(
                "Cannot resolve BASE: HOME environment variable is not set or is empty. "
                "Pass home= explicitly or set HOME."
            )
        home_path = Path(home_env)
    tmp_path = Path(tmp_candidate) if tmp_candidate is not None else _TMP_CANDIDATE

    claude_subdir = home_path / ".claude"
    cwd_under_home_claude = cwd_path == claude_subdir or _is_under(
        cwd_path, claude_subdir
    )
    cwd_is_home = cwd_path == home_path

    if not cwd_is_home and not cwd_under_home_claude:
        anchor_path = cwd_path / ANCHOR_FILENAME
        existing = _read_consistent_anchor(anchor_path)
        if existing is not None:
            return {
                "status": "resolved",
                "base": existing,
                "source": "anchor",
                "anchor": str(anchor_path),
            }
        _write_anchor(anchor_path, base=str(cwd_path), source="cwd-inference")
        return {
            "status": "resolved",
            "base": str(cwd_path),
            "source": "cwd-inference",
            "anchor": str(anchor_path),
        }

    candidates: list[Path] = [tmp_path, cwd_path]
    for candidate in candidates:
        anchor_path = candidate / ANCHOR_FILENAME
        existing = _read_consistent_anchor(anchor_path)
        if existing is not None:
            return {
                "status": "resolved",
                "base": existing,
                "source": "anchor",
                "anchor": str(anchor_path),
            }
    return {
        "status": "needs-user-input",
        "base": None,
        "source": None,
        "candidates": [str(c) for c in candidates],
    }


def record(base: str, *, source: str) -> Path:
    """Persist an anchor at ``<base>/.cafleet-base-dir.json`` (idempotent on match)."""
    if source not in ("cwd-inference", "askuserquestion"):
        raise ValueError(
            f"source must be 'cwd-inference' or 'askuserquestion'; got {source!r}"
        )
    base_dir = Path(base)
    if not base_dir.is_absolute():
        raise ValueError(f"base must be an absolute path; got {base!r}")
    # Normalize the string form so a caller-supplied trailing slash does not
    # desynchronize the JSON `base` field from the anchor's parent directory
    # (which `_read_consistent_anchor` later compares against).
    base = str(base_dir)
    anchor_path = base_dir / ANCHOR_FILENAME

    if anchor_path.is_file():
        data = _load_anchor(anchor_path)
        existing_base = data["base"]
        # Normalize via Path.resolve() so equivalent paths with different
        # lexical forms (symlinks, `..` segments, trailing slash) do not
        # break record() idempotency — matches _read_consistent_anchor.
        if Path(existing_base).resolve(strict=False) != Path(base).resolve(
            strict=False
        ):
            raise AnchorError(
                f"anchor file {anchor_path} records base={existing_base} "
                f"but lives at {base}; refusing to overwrite."
            )
        return anchor_path

    _write_anchor(anchor_path, base=base, source=source)
    return anchor_path


def is_git_repo_root(path: Path) -> bool:
    """True when ``<path>/.git`` exists (file or directory — supports worktrees)."""
    return (path / ".git").exists()


_FENCE_LINE_RE = re.compile(r"^```[A-Za-z0-9_+-]*$")


def extract_spawn_templates(content: str) -> list[str]:
    """Extract fenced spawn-prompt bodies from SKILL.md content.

    Uses a line-by-line state machine that pairs fenced-code opens and closes
    correctly across both bare and language-tagged fences. Accepted fence
    syntax is ``^```([A-Za-z0-9_+-]*)$`` — the three backticks may be
    immediately followed by a no-whitespace language tag (``bash``, ``text``,
    ``html``, ``markdown``, etc.) or by nothing. Fences with intervening
    whitespace or trailing characters are not recognized. A naive regex like
    ``` ``` \\n(.*?)\\n ``` ``` mismatches when bare close-fences of
    language-tagged blocks get treated as new openings; this parser tracks
    parity instead.

    A "spawn-prompt" body is identified by the simultaneous presence of:

    - the literal token ``YOUR AGENT ID: {agent_id}`` (curly placeholder
      injected by ``cafleet member create``'s ``str.format()`` pass), and
    - the ``BASE: [INSERT abs BASE path …]`` marker substring.
    """
    templates: list[str] = []
    in_block = False
    block_buf: list[str] = []
    for line in content.splitlines():
        if _FENCE_LINE_RE.match(line):
            if in_block:
                body = "\n".join(block_buf)
                if "YOUR AGENT ID: {agent_id}" in body and _BASE_INSERT_MARKER in body:
                    templates.append(body)
                block_buf = []
                in_block = False
            else:
                in_block = True
        elif in_block:
            block_buf.append(line)
    return templates


def substitute_base_in_prompt(prompt: str, *, base: str) -> str:
    """Substitute the BASE marker in a spawn-prompt template.

    When ``base`` is the ``<unset>`` sentinel, the entire ``BASE:`` line is
    removed — the literal string ``BASE: <unset>`` is never written into a
    spawn prompt (per §Specification 5 item 2 of design 0000055).
    """
    if base == UNSET_SENTINEL:
        lines = prompt.splitlines(keepends=True)
        return "".join(line for line in lines if _BASE_LINE_MARKER not in line)
    return prompt.replace(_BASE_LINE_MARKER, f"BASE: {base}")


def write_audit_file(*, base: str, filename: str, content: str) -> Path | None:
    """Write an audit file under BASE; skip and return ``None`` when BASE is ``<unset>``.

    This is the canonical Director-side audit-write helper: it guards every
    BASE-derived write at the call site so the `<unset>` sentinel never
    silently falls back to ``/tmp`` (per §Specification 5 item 1).
    """
    if base == UNSET_SENTINEL:
        return None
    base_dir = Path(base)
    if not base_dir.is_absolute():
        raise ValueError(f"base must be an absolute path; got {base!r}")
    audit_path = base_dir / filename
    audit_path.parent.mkdir(parents=True, exist_ok=True)
    audit_path.write_text(content, encoding="utf-8")
    return audit_path
