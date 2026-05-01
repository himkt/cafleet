"""Structural tests for Step 1 of design 0000045 (cafleet design-doc-interview port).

Step 1 is documentation-only. It updates two CLAUDE.md files to advertise the new
``cafleet:design-doc-interview`` skill and confirms ``cafleet:design-doc-create``
no longer marks interview as global-only. These tests assert the Step-1
specification verbatim, not implementation details.
"""

from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent

ROOT_CLAUDE_MD = PROJECT_ROOT / "CLAUDE.md"
DOTCLAUDE_CLAUDE_MD = PROJECT_ROOT / ".claude" / "CLAUDE.md"
DESIGN_DOC_CREATE_SKILL = PROJECT_ROOT / "skills" / "design-doc-create" / "SKILL.md"


@pytest.fixture
def root_claude_md_text() -> str:
    return ROOT_CLAUDE_MD.read_text(encoding="utf-8")


@pytest.fixture
def dotclaude_claude_md_text() -> str:
    return DOTCLAUDE_CLAUDE_MD.read_text(encoding="utf-8")


@pytest.fixture
def design_doc_create_skill_text() -> str:
    return DESIGN_DOC_CREATE_SKILL.read_text(encoding="utf-8")


def _interview_entry_lines(claude_md_text: str) -> list[str]:
    """Return the markdown bullet lines that mention design-doc-interview."""
    return [
        line
        for line in claude_md_text.splitlines()
        if line.lstrip().startswith("- ") and "design-doc-interview" in line
    ]


def test_root_claude_md_exists():
    assert ROOT_CLAUDE_MD.is_file(), f"missing {ROOT_CLAUDE_MD}"


def test_dotclaude_claude_md_exists():
    assert DOTCLAUDE_CLAUDE_MD.is_file(), f"missing {DOTCLAUDE_CLAUDE_MD}"


def test_design_doc_create_skill_exists():
    assert DESIGN_DOC_CREATE_SKILL.is_file(), f"missing {DESIGN_DOC_CREATE_SKILL}"


def test_root_claude_md_lists_design_doc_interview_skill(root_claude_md_text):
    """Design 0000045 Step 1.a: root CLAUDE.md lists /cafleet:design-doc-interview as a skill bullet."""
    entries = _interview_entry_lines(root_claude_md_text)
    assert entries, (
        "root CLAUDE.md has no markdown bullet referencing design-doc-interview; "
        "design 0000045 Step 1.a requires the skill to be listed in Project Skills"
    )


def test_root_claude_md_interview_entry_mentions_comment_marker(root_claude_md_text):
    """Design 0000045 Step 1.a: entry must mention COMMENT(claude) annotations."""
    entries = _interview_entry_lines(root_claude_md_text)
    assert entries, "no design-doc-interview bullet to inspect"
    blob = "\n".join(entries)
    assert "COMMENT(claude)" in blob, (
        "root CLAUDE.md design-doc-interview entry must mention 'COMMENT(claude)' "
        "(the marker consumed by /design-doc-create resume mode)"
    )


def test_root_claude_md_interview_entry_references_create_skill(root_claude_md_text):
    """Design 0000045 Step 1.a: entry must reference /design-doc-create as the upstream step."""
    entries = _interview_entry_lines(root_claude_md_text)
    blob = "\n".join(entries)
    assert "design-doc-create" in blob, (
        "root CLAUDE.md design-doc-interview entry must reference /design-doc-create"
    )


def test_root_claude_md_interview_entry_references_execute_skill(root_claude_md_text):
    """Design 0000045 Step 1.a: entry must reference /design-doc-execute as the downstream step."""
    entries = _interview_entry_lines(root_claude_md_text)
    blob = "\n".join(entries)
    assert "design-doc-execute" in blob, (
        "root CLAUDE.md design-doc-interview entry must reference /design-doc-execute"
    )


def test_dotclaude_claude_md_lists_design_doc_interview_skill(dotclaude_claude_md_text):
    """Design 0000045 Step 1.b: .claude/CLAUDE.md mirrors the root entry."""
    entries = _interview_entry_lines(dotclaude_claude_md_text)
    assert entries, (
        ".claude/CLAUDE.md has no markdown bullet referencing design-doc-interview; "
        "design 0000045 Step 1.b requires the same addition mirrored in the .claude/ copy"
    )


def test_dotclaude_claude_md_interview_entry_mentions_comment_marker(dotclaude_claude_md_text):
    """Design 0000045 Step 1.b: mirrored entry retains the COMMENT(claude) reference."""
    entries = _interview_entry_lines(dotclaude_claude_md_text)
    assert entries, "no design-doc-interview bullet to inspect in .claude/CLAUDE.md"
    blob = "\n".join(entries)
    assert "COMMENT(claude)" in blob, (
        ".claude/CLAUDE.md design-doc-interview entry must mention 'COMMENT(claude)'"
    )


def test_design_doc_create_skill_does_not_mark_interview_as_global_only(
    design_doc_create_skill_text,
):
    """Design 0000045 Step 1.c: design-doc-create SKILL.md must not call interview global-only."""
    haystack = design_doc_create_skill_text.lower()
    forbidden_phrases = [
        "interview is only available globally",
        "interview is global-only",
        "global-only interview",
        "interview only exists globally",
        "interview is not available in cafleet",
        "interview skill is global",
        "no cafleet interview",
    ]
    matches = [phrase for phrase in forbidden_phrases if phrase in haystack]
    assert not matches, (
        f"skills/design-doc-create/SKILL.md still flags interview as global-only: {matches!r}; "
        "design 0000045 Step 1.c requires this language to be removed"
    )
