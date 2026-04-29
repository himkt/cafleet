"""Permission-aware shell dispatch for ``cafleet member safe-exec``.

Pure utility module with zero coupling to the broker, the CLI, or
``Settings``. Owns settings discovery (`discover_settings_paths`),
file loading and union semantics (`load_bash_patterns`), the
glob matcher (`match`), and the allow/deny/ask decision (`decide`).
Every call re-reads disk; nothing is cached.
"""

from __future__ import annotations

import json
import os
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Literal


@dataclass(frozen=True)
class Pattern:
    raw: str
    body: str
    source_file: Path


@dataclass(frozen=True)
class Decision:
    outcome: Literal["allow", "deny", "ask"]
    matched_pattern: str | None
    matched_file: Path | None
    offending_substring: str | None
    searched_files: list[Path]


def discover_settings_paths() -> list[Path]:
    cwd = Path.cwd()
    project_local = cwd / ".claude" / "settings.local.json"
    project_shared = cwd / ".claude" / "settings.json"
    config_dir = os.environ.get("CLAUDE_CONFIG_DIR")
    if config_dir:
        user_settings = Path(config_dir) / "settings.json"
    else:
        user_settings = Path("~/.claude/settings.json").expanduser()
    return [project_local, project_shared, user_settings]


_BASH_PREFIX = "Bash("
_BASH_SUFFIX = ")"


def _extract_bash_body(entry: object) -> str | None:
    if not isinstance(entry, str):
        return None
    if not entry.startswith(_BASH_PREFIX) or not entry.endswith(_BASH_SUFFIX):
        return None
    return entry[len(_BASH_PREFIX) : -len(_BASH_SUFFIX)]


def _load_one(path: Path) -> tuple[list[Pattern], list[Pattern]]:
    if not path.exists():
        return [], []
    text = path.read_text()
    try:
        doc = json.loads(text)
    except json.JSONDecodeError as exc:
        raise ValueError(f"failed to parse {path}: {exc}") from exc
    if not isinstance(doc, dict):
        return [], []
    perms = doc.get("permissions")
    if not isinstance(perms, dict):
        return [], []
    allow_raw = perms.get("allow", [])
    deny_raw = perms.get("deny", [])
    allow: list[Pattern] = []
    deny: list[Pattern] = []
    if isinstance(allow_raw, list):
        for entry in allow_raw:
            body = _extract_bash_body(entry)
            if body is not None:
                allow.append(Pattern(raw=entry, body=body, source_file=path))
    if isinstance(deny_raw, list):
        for entry in deny_raw:
            body = _extract_bash_body(entry)
            if body is not None:
                deny.append(Pattern(raw=entry, body=body, source_file=path))
    return allow, deny


def load_bash_patterns(paths: list[Path]) -> tuple[list[Pattern], list[Pattern]]:
    all_allow: list[Pattern] = []
    all_deny: list[Pattern] = []
    for path in paths:
        allow, deny = _load_one(path)
        all_allow.extend(allow)
        all_deny.extend(deny)
    return all_allow, all_deny


def _compile_body(body: str) -> re.Pattern[str]:
    if body == "*":
        return re.compile(".*", re.DOTALL)

    if body.endswith((" *", ":*")):
        prefix_body = body[:-2]
        trailing = "(?:\\s.*)?"
    elif body.endswith("*") and len(body) >= 2 and body[-2] not in (" ", ":"):
        prefix_body = body[:-1]
        trailing = ".*"
    else:
        prefix_body = body
        trailing = ""

    parts: list[str] = []
    for ch in prefix_body:
        if ch == "*":
            parts.append(".*")
        else:
            parts.append(re.escape(ch))
    regex = "".join(parts) + trailing
    return re.compile(regex, re.DOTALL)


def match(pattern: Pattern, command: str) -> bool:
    return _compile_body(pattern.body).fullmatch(command) is not None


def decide(command: str, paths: list[Path]) -> Decision:
    allow, deny = load_bash_patterns(paths)
    for pat in deny:
        if match(pat, command):
            return Decision(
                outcome="deny",
                matched_pattern=pat.raw,
                matched_file=pat.source_file,
                offending_substring=command,
                searched_files=list(paths),
            )
    for pat in allow:
        if match(pat, command):
            return Decision(
                outcome="allow",
                matched_pattern=pat.raw,
                matched_file=pat.source_file,
                offending_substring=command,
                searched_files=list(paths),
            )
    return Decision(
        outcome="ask",
        matched_pattern=None,
        matched_file=None,
        offending_substring=None,
        searched_files=list(paths),
    )
