"""Static drift guard for the spawn-prompt / CLI-name reconciliation (design 0000113 S4).

Scans the project-local edit surface — the repo ``skills/`` tree, ``docs/``,
``SPEC.md``, ``README.md``, the project-local ``.claude/`` (``rules/`` +
``skills/skill-author/``), and ``CLAUDE.md`` — excluding ``design-docs/``
(the historical record) — and reports a violation naming the file, line, and
matched pattern for any occurrence of:

1. ``$CAFLEET_FLEET_ID`` / ``$CAFLEET_AGENT_ID`` / ``$CAFLEET_DIRECTOR_AGENT_ID``
   — dollar-sign identity env refs (the fiction issue #155 removes); the bare
   dollar-less names in the CLI env-var catalog are NOT matched.
2. ``cafleet agent spawn`` — removed command; now ``cafleet member create``.
3. ``cafleet pane `` (with trailing space / subcommand) — removed group; now
   ``cafleet member *``.

Matching is plain per-line substring containment (one violation per matched
pattern per line), so occurrences inside inline code spans and fenced blocks
are flagged like any other text.

Wrapped by both ``cafleet/tests/test_spawn_prompt_guard.py`` (which runs
inside ``mise //cafleet:test``) and the ``mise //cafleet:lint-spawn-guard``
task (the ``python -m cafleet.spawn_prompt_guard`` entry point below).
"""

import sys
from pathlib import Path

FORBIDDEN_PATTERNS: tuple[str, ...] = (
    "$CAFLEET_FLEET_ID",
    "$CAFLEET_AGENT_ID",
    "$CAFLEET_DIRECTOR_AGENT_ID",
    "cafleet agent spawn",
    "cafleet pane ",
)

# The S3/S4 project-local edit surface, relative to the repo root. Directories
# are scanned recursively; missing entries are tolerated (scan what exists).
_SURFACE_DIRS: tuple[str, ...] = (
    "skills",
    "docs",
    ".claude/rules",
    ".claude/skills/skill-author",
)
_SURFACE_FILES: tuple[str, ...] = ("SPEC.md", "README.md", "CLAUDE.md")


def scan_text(rel_path: str, text: str) -> list[str]:
    """Scan one file's text; return ``<rel_path>:<line>`` violations.

    Pure: one violation per (line, pattern) match, each naming the file, the
    1-based line number, and the matched pattern.
    """
    return [
        f"{rel_path}:{lineno}: forbidden pattern '{pattern}'"
        for lineno, line in enumerate(text.splitlines(), start=1)
        for pattern in FORBIDDEN_PATTERNS
        if pattern in line
    ]


def _repo_root() -> Path:
    """Locate the repo root (the directory holding ``skills/cafleet/SKILL.md``)
    relative to this module."""
    for parent in Path(__file__).resolve().parents:
        if (parent / "skills" / "cafleet" / "SKILL.md").is_file():
            return parent
    raise FileNotFoundError("could not locate the repo root from spawn_prompt_guard.py")


def _surface_files(root: Path) -> list[Path]:
    files: list[Path] = []
    for rel_dir in _SURFACE_DIRS:
        directory = root / rel_dir
        if directory.is_dir():
            files.extend(
                sorted(path for path in directory.rglob("*") if path.is_file())
            )
    for rel_file in _SURFACE_FILES:
        path = root / rel_file
        if path.is_file():
            files.append(path)
    return files


def check_spawn_prompt_drift(root: Path | None = None) -> list[str]:
    """Scan the project-local edit surface under ``root`` (default: the live
    repo root); return all violations."""
    resolved_root = _repo_root() if root is None else Path(root)
    violations: list[str] = []
    for path in _surface_files(resolved_root):
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        rel_path = path.relative_to(resolved_root).as_posix()
        violations.extend(scan_text(rel_path, text))
    return violations


def main() -> int:
    """Entry point for ``mise //cafleet:lint-spawn-guard``."""
    violations = check_spawn_prompt_drift()
    if violations:
        print("spawn-prompt guard: FAIL")
        for violation in violations:
            print(f"  - {violation}")
        return 1
    print("spawn-prompt guard: OK — project-local edit surface is drift-free")
    return 0


if __name__ == "__main__":
    sys.exit(main())
