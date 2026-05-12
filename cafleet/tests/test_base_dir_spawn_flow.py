"""Integration tests for the design-0000055 spawn-prompt + audit-write contract.

These tests exercise:

1. The Director-side substitution that turns the `[INSERT abs BASE path ...]`
   marker in each spawn template into a concrete `BASE: <abs path>` line.
2. The matching elision when BASE is the `<unset>` sentinel (the line is
   omitted entirely — never rendered as `BASE: <unset>`).
3. The Director-side audit-file write helper, which derives an output path
   from BASE and refuses to fall back to `/tmp/claude-code` when BASE points
   somewhere else.

End-to-end member spawning (`cafleet member create` + real Claude API) is
explicitly out of scope per the design doc.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

from cafleet.base_dir import (
    UNSET_SENTINEL,
    record,
    substitute_base_in_prompt,
    write_audit_file,
)


REPO_ROOT = Path(__file__).resolve().parents[2]
SKILLS_DIR = REPO_ROOT / "skills"

CONSUMER_SKILLS = [
    SKILLS_DIR / "research-report" / "SKILL.md",
    SKILLS_DIR / "research-presentation" / "SKILL.md",
    SKILLS_DIR / "design-doc-create" / "SKILL.md",
    SKILLS_DIR / "design-doc-execute" / "SKILL.md",
    SKILLS_DIR / "design-doc-interview" / "SKILL.md",
]

BASE_MARKER = (
    "[INSERT abs BASE path the Director resolved via Skill(cafleet:base-dir)]"
)

# Total number of spawn templates that gain a BASE line (design doc §Spec 2):
#   research-report:        Manager, Scout, Researcher                 (3)
#   research-presentation:  Presentation, Transcript, Visual Reviewer  (3)
#   design-doc-create:      Drafter (normal), Drafter (resume), Reviewer (3)
#   design-doc-execute:     Programmer, Tester, Verifier               (3)
#   design-doc-interview:   Analyzer                                   (1)
EXPECTED_SPAWN_TEMPLATE_COUNT = 13


def _extract_spawn_templates(skill_md: Path) -> list[str]:
    """Return every fenced code block in ``skill_md`` whose body is a spawn prompt.

    A "spawn-prompt" code block is identified by the simultaneous presence of:
    - the literal token ``YOUR AGENT ID: {agent_id}`` (curly placeholder injected
      by ``cafleet member create``'s ``str.format()`` pass), and
    - the ``[INSERT BASE]`` marker on a ``BASE:`` line.
    """
    content = skill_md.read_text(encoding="utf-8")
    templates: list[str] = []
    for match in re.finditer(r"```\n(.*?)\n```", content, flags=re.DOTALL):
        body = match.group(1)
        if "YOUR AGENT ID: {agent_id}" in body and BASE_MARKER in body:
            templates.append(body)
    return templates


def _all_spawn_templates() -> list[tuple[Path, str]]:
    """Collect every spawn template across the 5 consumer SKILL.md files."""
    out: list[tuple[Path, str]] = []
    for skill_md in CONSUMER_SKILLS:
        for tpl in _extract_spawn_templates(skill_md):
            out.append((skill_md, tpl))
    return out


# ---------------------------------------------------------------------------


def test_spawn_prompt_substitution_carries_base_to_member():
    """A concrete BASE substitutes into every affected spawn template.

    Asserts the rendered prompt has a ``BASE: <abs path>`` line positioned
    immediately after the ``YOUR AGENT ID: {agent_id}`` line — the stable
    insertion position required by §Specification 2 *Insertion position*.
    """
    templates = _all_spawn_templates()
    assert len(templates) == EXPECTED_SPAWN_TEMPLATE_COUNT, (
        f"expected {EXPECTED_SPAWN_TEMPLATE_COUNT} spawn templates with BASE markers, "
        f"found {len(templates)} (per-file count: "
        f"{[(p.parent.name, len(_extract_spawn_templates(p))) for p in CONSUMER_SKILLS]})"
    )

    base = "/home/user/myproject"

    for skill_md, template in templates:
        rendered = substitute_base_in_prompt(template, base=base)
        lines = rendered.splitlines()
        try:
            agent_idx = next(
                i for i, line in enumerate(lines)
                if line.startswith("YOUR AGENT ID:")
            )
        except StopIteration:
            pytest.fail(
                f"{skill_md.parent.name}: rendered prompt lost its YOUR AGENT ID: line"
            )

        assert agent_idx + 1 < len(lines), (
            f"{skill_md.parent.name}: BASE line missing after YOUR AGENT ID: "
            f"(template ended)"
        )
        assert lines[agent_idx + 1] == f"BASE: {base}", (
            f"{skill_md.parent.name}: BASE line not immediately after YOUR AGENT ID: "
            f"(got {lines[agent_idx + 1]!r})"
        )
        assert BASE_MARKER not in rendered, (
            f"{skill_md.parent.name}: the [INSERT BASE] marker survived substitution"
        )


def test_spawn_prompt_omits_base_line_when_unset():
    """When BASE is <unset>, the entire BASE: line is removed from each rendered prompt.

    The literal string ``BASE: <unset>`` is never written into a spawn prompt
    (§Specification 2 / §Specification 5 item 2).
    """
    templates = _all_spawn_templates()
    assert len(templates) == EXPECTED_SPAWN_TEMPLATE_COUNT

    for skill_md, template in templates:
        rendered = substitute_base_in_prompt(template, base=UNSET_SENTINEL)

        for line in rendered.splitlines():
            assert not line.startswith("BASE:"), (
                f"{skill_md.parent.name}: BASE: line still present after unset "
                f"substitution: {line!r}"
            )
        assert "<unset>" not in rendered, (
            f"{skill_md.parent.name}: literal '<unset>' leaked into prompt"
        )
        assert BASE_MARKER not in rendered, (
            f"{skill_md.parent.name}: the [INSERT BASE] marker survived unset elision"
        )

        # The surrounding identity block must remain intact: YOUR AGENT ID: stays.
        assert any(
            line.startswith("YOUR AGENT ID:") for line in rendered.splitlines()
        ), f"{skill_md.parent.name}: YOUR AGENT ID: line missing post-elision"


def test_director_side_audit_writes_land_under_base_not_tmp(tmp_path):
    """The Director-side audit-file write helper lands files under BASE only.

    Scenario: an anchor exists at a non-`/tmp/claude-code` BASE (the pytest
    tmp_path fixture). The helper writes the audit file there, and nothing
    appears under `/tmp/claude-code/`. The `<unset>` case also returns None
    without writing.
    """
    base = tmp_path / "myproj"
    base.mkdir()
    record(str(base), source="askuserquestion")

    # Snapshot the recommended-default location so we can prove the helper
    # never accidentally polluted it.
    tmp_claude = Path("/tmp/claude-code")
    before = set(tmp_claude.rglob("*")) if tmp_claude.exists() else set()

    audit_path = write_audit_file(
        base=str(base),
        filename="drafter.md",
        content="rendered spawn prompt audit\n",
    )

    assert audit_path == base / "drafter.md"
    assert audit_path.is_file()
    assert audit_path.read_text() == "rendered spawn prompt audit\n"

    after = set(tmp_claude.rglob("*")) if tmp_claude.exists() else set()
    new_paths = after - before
    assert not new_paths, (
        f"audit-write helper leaked files into /tmp/claude-code: {sorted(new_paths)}"
    )

    # The <unset> branch: helper must return None and write nothing.
    skipped = write_audit_file(
        base=UNSET_SENTINEL,
        filename="drafter.md",
        content="should not appear anywhere",
    )
    assert skipped is None
    after_unset = set(tmp_claude.rglob("*")) if tmp_claude.exists() else set()
    assert after_unset - after == set(), (
        "audit-write helper wrote under /tmp/claude-code despite BASE=<unset>"
    )
