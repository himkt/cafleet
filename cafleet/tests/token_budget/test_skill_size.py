"""Skill-file size budget.

The core ``skills/cafleet/SKILL.md`` is loaded into context every time a
member spawns. The core file is kept under ≤ 350 lines with on-demand
reference files alongside it. The budget below catches a re-merge /
accidental re-import of reference content into the core.
"""

from pathlib import Path

_REPO_ROOT = Path(__file__).resolve().parents[3]
_SKILL_FILE = _REPO_ROOT / "skills" / "cafleet" / "SKILL.md"


def _line_count(path: Path) -> int:
    with path.open("r", encoding="utf-8") as fh:
        return sum(1 for _ in fh)


def test_core_skill_file_exists():
    """Sanity check — the budget assertion below would falsely pass if the
    file had been moved or renamed without updating this test path."""
    assert _SKILL_FILE.is_file(), f"expected skill file at {_SKILL_FILE}"


def test_core_skill_file_under_350_lines():
    """≤ 350 lines for the core file. Blowing past 350 means we re-merged
    reference content (director.md, broadcast.md, exec-routing.md,
    recovery.md, legacy-flags.md) back into the core file."""
    line_count = _line_count(_SKILL_FILE)
    assert line_count <= 350, (
        f"skills/cafleet/SKILL.md grew to {line_count} lines (budget 350); "
        f"check whether reference content was re-merged into the core file"
    )


def test_core_skill_file_under_25kb():
    """Per-byte budget. 25 KB is comfortably above a 350-line file with
    long-line reference tables; a regression past it suggests a large
    fenced code block or copy-pasted reference section landed in the core."""
    size_bytes = _SKILL_FILE.stat().st_size
    assert size_bytes <= 25_000, (
        f"skills/cafleet/SKILL.md grew to {size_bytes} bytes (budget 25000)"
    )


def test_reference_files_exist():
    """The split's reference files MUST exist alongside the slim core so
    members can Read them on demand. A regression that deletes a reference
    file would force the content back into the core (tripping the line
    budget above) — but this test catches it earlier with a clearer
    diagnostic."""
    expected_refs = (
        "director.md",
        "broadcast.md",
        "exec-routing.md",
        "recovery.md",
        "legacy-flags.md",
    )
    reference_dir = _REPO_ROOT / "skills" / "cafleet" / "reference"
    for name in expected_refs:
        path = reference_dir / name
        assert path.is_file(), (
            f"missing reference file {path} — all five reference files must "
            f"sit alongside the slim core"
        )
