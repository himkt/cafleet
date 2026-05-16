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

from pathlib import Path

import pytest

from cafleet.base_dir import (
    ANCHOR_FILENAME,
    UNSET_SENTINEL,
    extract_spawn_templates,
    record,
    resolve,
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

BASE_MARKER = "[INSERT abs BASE path the Director resolved via Skill(cafleet:base-dir)]"

# Total number of spawn templates that gain a BASE line (design doc §Spec 2):
#   research-report:        Manager, Scout, Researcher                 (3)
#   research-presentation:  Presentation, Transcript, Visual Reviewer  (3)
#   design-doc-create:      Drafter (normal), Drafter (resume), Reviewer (3)
#   design-doc-execute:     Programmer, Tester, Verifier               (3)
#   design-doc-interview:   Analyzer                                   (1)
EXPECTED_SPAWN_TEMPLATE_COUNT = 13


def _extract_spawn_templates(skill_md: Path) -> list[str]:
    """Return every fenced spawn-prompt body in ``skill_md``.

    Thin wrapper that delegates to :func:`cafleet.base_dir.extract_spawn_templates`
    — the line-by-line fence-parity parser that handles both bare ` ``` ` opens
    and language-tagged opens (` ```bash`, ` ```text`, etc.). A regex-only
    parser silently mis-pairs fences and returns the wrong bodies.
    """
    return extract_spawn_templates(skill_md.read_text(encoding="utf-8"))


def _all_spawn_templates() -> list[tuple[Path, str]]:
    """Collect every spawn template across the 5 consumer SKILL.md files."""
    return [
        (skill_md, tpl)
        for skill_md in CONSUMER_SKILLS
        for tpl in _extract_spawn_templates(skill_md)
    ]


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
                i for i, line in enumerate(lines) if line.startswith("YOUR AGENT ID:")
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

    Asserted structurally: the helper writes precisely one file at
    ``base/filename`` (no second write site, no /tmp fallback). The
    ``<unset>`` case is a guarded skip that returns None without writing.
    """
    base = tmp_path / "myproj"
    base.mkdir()
    record(str(base), source="askuserquestion")
    anchor_path = base / ".cafleet-base-dir.json"

    audit_path = write_audit_file(
        base=str(base),
        filename="drafter.md",
        content="rendered spawn prompt audit\n",
    )

    # The audit file lands at the documented BASE-relative path.
    assert audit_path == base / "drafter.md"
    assert audit_path.is_file()
    assert audit_path.read_text() == "rendered spawn prompt audit\n"

    # No other files appear under BASE — only the audit file and the anchor.
    files_under_base = {p for p in base.rglob("*") if p.is_file()}
    expected = {audit_path, anchor_path}
    assert files_under_base == expected, (
        f"unexpected files under BASE: extras={sorted(files_under_base - expected)}, "
        f"missing={sorted(expected - files_under_base)}"
    )

    # The <unset> branch: helper must return None and write nothing extra.
    skipped = write_audit_file(
        base=UNSET_SENTINEL,
        filename="drafter.md",
        content="should not appear anywhere",
    )
    assert skipped is None
    files_after_unset = {p for p in base.rglob("*") if p.is_file()}
    assert files_after_unset == expected, (
        "audit-write helper wrote a file despite BASE=<unset>"
    )


# ---------------------------------------------------------------------------
# Design 0000060 — task-scoped spawn-prompt audit-write flow
# ---------------------------------------------------------------------------


def test_task_scoped_spawn_audit_lands_in_task_folder_not_repo_root(tmp_path):
    """End-to-end: a fake consuming skill resolves a task-scoped BASE, substitutes it into
    a fenced spawn-prompt template, and persists the rendered prompt as an audit file. The
    audit file MUST land at ``<task-folder>/prompts/<role>-<ts>.md`` — never at
    ``<repo-root>/prompts/``."""
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    (repo_root / ".git").mkdir()

    # 1. Director resolves the task-scoped BASE for this run.
    result = resolve(task_name="design-docs/0000099-end-to-end", cwd=repo_root)
    assert result["status"] == "resolved"
    base = result["base"]
    expected_task_folder = repo_root / "design-docs" / "0000099-end-to-end"
    assert base == str(expected_task_folder)
    assert result["source"] == "task-scope"
    assert (expected_task_folder / ANCHOR_FILENAME).exists()

    # 2. The consuming skill's SKILL.md contains a fenced spawn-prompt template.
    skill_md_body = (
        "Some intro markdown...\n"
        "\n"
        "```text\n"
        "SESSION ID: {session_id}\n"
        "DIRECTOR AGENT ID: {director_agent_id}\n"
        "YOUR AGENT ID: {agent_id}\n"
        "BASE: [INSERT abs BASE path the Director resolved via Skill(cafleet:base-dir)]\n"
        "ROLE: drafter\n"
        "...body of the spawn prompt...\n"
        "```\n"
        "\n"
        "More trailing markdown.\n"
    )
    templates = extract_spawn_templates(skill_md_body)
    assert len(templates) == 1, "expected exactly one extracted spawn template"

    # 3. Director substitutes the task-scoped BASE into the rendered prompt.
    rendered = substitute_base_in_prompt(templates[0], base=base)
    assert f"BASE: {base}" in rendered
    assert BASE_MARKER not in rendered

    # 4. Persist the rendered spawn prompt as an audit file under BASE/prompts/.
    audit_filename = "prompts/drafter-2026-05-16T12-00-00.md"
    audit_path = write_audit_file(base=base, filename=audit_filename, content=rendered)

    # 5. The audit file lives under the task folder, NOT the repo root.
    expected_audit_path = (
        expected_task_folder / "prompts" / "drafter-2026-05-16T12-00-00.md"
    )
    assert audit_path == expected_audit_path
    assert audit_path.is_file()
    assert audit_path.read_text() == rendered

    # The repo-root prompts/ directory must NOT exist — no bypass to repo root.
    assert not (repo_root / "prompts").exists()
