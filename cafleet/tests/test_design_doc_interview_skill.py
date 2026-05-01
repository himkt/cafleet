"""Structural tests for design 0000045 (cafleet design-doc-interview port).

The design document is documentation-only: it adds skill markdown files and
edits two CLAUDE.md files. These tests assert the design spec verbatim
(file presence, YAML front-matter validity, required-section presence,
COMMENT-marker compatibility, CLAUDE.md skill-list entries) — not the
contents of any prior untrusted draft.
"""

from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent

ROOT_CLAUDE_MD = PROJECT_ROOT / "CLAUDE.md"
DOTCLAUDE_CLAUDE_MD = PROJECT_ROOT / ".claude" / "CLAUDE.md"
DESIGN_DOC_CREATE_SKILL = PROJECT_ROOT / "skills" / "design-doc-create" / "SKILL.md"
SKILL_DIR = PROJECT_ROOT / "skills" / "design-doc-interview"
SKILL_MD = SKILL_DIR / "SKILL.md"
ANALYZER_MD = SKILL_DIR / "roles" / "analyzer.md"


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


# ---------------------------------------------------------------------------
# Step 2: Skill scaffolding (skills/design-doc-interview/SKILL.md + analyzer.md)
# ---------------------------------------------------------------------------


def _split_front_matter(text: str) -> tuple[dict[str, str], str]:
    """Parse a leading ``---``-delimited YAML block by hand (no PyYAML dep).

    Returns ``(front_matter_dict, body)``. Raises AssertionError if no
    front-matter block is present or it is not properly closed.
    """
    lines = text.splitlines()
    assert lines and lines[0].strip() == "---", (
        "front-matter must begin with a '---' line on the very first line"
    )
    closing = None
    for index in range(1, len(lines)):
        if lines[index].strip() == "---":
            closing = index
            break
    assert closing is not None, (
        "front-matter is not closed — expected a second '---' line"
    )
    body = "\n".join(lines[closing + 1 :])
    front_matter: dict[str, str] = {}
    for raw in lines[1:closing]:
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        assert ":" in raw, f"front-matter line missing ':': {raw!r}"
        key, _, value = raw.partition(":")
        front_matter[key.strip()] = value.strip()
    return front_matter, body


@pytest.fixture
def skill_md_text() -> str:
    return SKILL_MD.read_text(encoding="utf-8")


@pytest.fixture
def analyzer_md_text() -> str:
    return ANALYZER_MD.read_text(encoding="utf-8")


def test_skill_dir_exists():
    """Design 0000045 Step 2.a: skills/design-doc-interview/ exists as a directory."""
    assert SKILL_DIR.is_dir(), f"missing skill directory: {SKILL_DIR}"


def test_skill_md_exists():
    """Design 0000045 Step 2.b: SKILL.md exists."""
    assert SKILL_MD.is_file(), f"missing skill file: {SKILL_MD}"


def test_skill_md_front_matter_has_required_keys(skill_md_text):
    """Design 0000045 Step 2.b: front-matter has name=design-doc-interview, description, allowed-tools."""
    front_matter, _body = _split_front_matter(skill_md_text)
    assert "name" in front_matter, "front-matter missing 'name'"
    assert "description" in front_matter, "front-matter missing 'description'"
    assert "allowed-tools" in front_matter, "front-matter missing 'allowed-tools'"
    assert front_matter["name"] == "design-doc-interview", (
        f"front-matter name must be 'design-doc-interview', got {front_matter['name']!r}"
    )
    assert front_matter["description"], "front-matter description must not be empty"
    assert front_matter["allowed-tools"], "front-matter allowed-tools must not be empty"


def test_skill_md_front_matter_allowed_tools_is_exact_six_tool_set(skill_md_text):
    """Design 0000045 Step 2.b: allowed-tools must be exactly the 6 tools listed in the spec.

    The design document specifies: ``Read, Write, Edit, Grep, AskUserQuestion, Bash``.
    Comparison is order-independent (set equality) but allows no extras and no omissions.
    """
    front_matter, _body = _split_front_matter(skill_md_text)
    raw = front_matter["allowed-tools"]
    actual = {tool.strip() for tool in raw.split(",") if tool.strip()}
    expected = {"Read", "Write", "Edit", "Grep", "AskUserQuestion", "Bash"}
    assert actual == expected, (
        f"front-matter allowed-tools must equal {sorted(expected)} (order-independent). "
        f"Got {sorted(actual)}. Extras: {sorted(actual - expected)}. "
        f"Missing: {sorted(expected - actual)}."
    )


def test_skill_md_body_has_process_section_and_step_headings(skill_md_text):
    """Design 0000045 Step 2.b: body has Process section heading + Step 0..Step 5 sub-headings."""
    _front_matter, body = _split_front_matter(skill_md_text)
    body_lower = body.lower()
    assert "## process" in body_lower, "missing top-level '## Process' heading"
    for step_index in range(6):  # 0..5 inclusive
        needle = f"### step {step_index}"
        assert needle in body_lower, (
            f"missing '### Step {step_index}' sub-heading under Process section"
        )


def test_skill_md_body_has_comment_annotation_format_section(skill_md_text):
    """Design 0000045 Step 2.b: body has a COMMENT annotation format section."""
    _front_matter, body = _split_front_matter(skill_md_text)
    lower = body.lower()
    assert "comment annotation format" in lower or "comment(claude) annotation format" in lower, (
        "missing 'COMMENT Annotation Format' section heading"
    )
    assert "COMMENT(claude)" in body, (
        "COMMENT annotation section must include the literal 'COMMENT(claude)' marker — "
        "this is what /design-doc-create resume mode greps for"
    )


def test_skill_md_body_has_question_md_format_section(skill_md_text):
    """Design 0000045 Step 2.b: body has a question.md format section."""
    _front_matter, body = _split_front_matter(skill_md_text)
    lower = body.lower()
    assert "question.md" in lower, "body must reference question.md"
    has_section_heading = any(
        line.lstrip().startswith("##") and "question.md" in line.lower()
        for line in body.splitlines()
    )
    assert has_section_heading, (
        "missing a markdown heading (## or ###) that names 'question.md' — design 0000045 requires "
        "a dedicated question.md format section"
    )
    assert "interview-progress" in lower, (
        "question.md format section must document the <!-- interview-progress: [...] --> marker"
    )


def test_skill_md_ends_with_arguments_footer(skill_md_text):
    """Design 0000045 Step 2.b: SKILL.md body ends with the literal $ARGUMENTS token."""
    stripped = skill_md_text.rstrip()
    assert stripped.endswith("$ARGUMENTS"), (
        f"SKILL.md must end with the $ARGUMENTS footer (Claude Code skill template injection); "
        f"last 40 chars are {stripped[-40:]!r}"
    )


def test_analyzer_md_exists():
    """Design 0000045 Step 2.c: analyzer.md exists."""
    assert ANALYZER_MD.is_file(), f"missing analyzer role file: {ANALYZER_MD}"


def test_analyzer_md_describes_required_role_behaviors(analyzer_md_text):
    """Design 0000045 Step 2.c: role file describes read-doc, return numbered list, idle pending shutdown."""
    lower = analyzer_md_text.lower()
    assert "design document" in lower or "design doc" in lower, (
        "analyzer.md must instruct the Analyzer to read the design document"
    )
    assert "read" in lower, "analyzer.md must describe reading"
    assert "numbered" in lower and ("question" in lower or "list" in lower), (
        "analyzer.md must describe returning a numbered question list"
    )
    assert "idle" in lower and "shutdown" in lower, (
        "analyzer.md must describe idling pending shutdown"
    )


# ---------------------------------------------------------------------------
# Step 3: Analyzer prompt template (categories, priority order, footer, brace safety)
# ---------------------------------------------------------------------------


_REQUIRED_CATEGORIES = (
    "Intent alignment",
    "Ambiguity",
    "Missing requirements",
    "Implicit assumptions",
    "Design decisions",
    "Internal consistency",
    "Implementation actionability",
)

_REQUIRED_PRIORITY_ITEMS = (
    "intent confirmation",
    "ambiguous",
    "implicit assumptions",
    "missing requirements",
    "design challenges",
    "implementation clarity",
)


def _spawn_prompt_block(skill_text: str) -> str:
    """Return the Step 2d spawn-prompt fenced code block, sans the fences."""
    lines = skill_text.splitlines()
    step_2d_index = next(
        (i for i, line in enumerate(lines) if line.startswith("#### 2d")),
        None,
    )
    assert step_2d_index is not None, "SKILL.md missing '#### 2d' sub-heading"

    fence_starts = [
        i
        for i in range(step_2d_index, len(lines))
        if lines[i].lstrip().startswith("```")
    ]
    assert len(fence_starts) >= 2, (
        "Step 2d section must contain at least one fenced code block (open + close fences)"
    )
    open_idx = fence_starts[0]
    close_idx = fence_starts[1]
    body = "\n".join(lines[open_idx + 1 : close_idx])
    return body


def test_analyzer_md_contains_all_seven_question_categories(analyzer_md_text):
    """Design 0000045 Step 3.a: analyzer.md must list all 7 question-generation categories."""
    lower = analyzer_md_text.lower()
    missing = [name for name in _REQUIRED_CATEGORIES if name.lower() not in lower]
    assert not missing, (
        f"analyzer.md is missing required category name(s): {missing!r}; "
        f"all 7 must appear: {list(_REQUIRED_CATEGORIES)!r}"
    )


def test_analyzer_md_documents_priority_order_with_six_items(analyzer_md_text):
    """Design 0000045 Step 3.a: analyzer.md must document the 6-item priority ordering."""
    lower = analyzer_md_text.lower()
    has_priority_heading_or_label = (
        "priority order" in lower
        or "priority ordering" in lower
        or "priority:" in lower
    )
    assert has_priority_heading_or_label, (
        "analyzer.md must include a 'Priority order' heading or label so the priority "
        "ranking is visible to the reader"
    )
    missing = [item for item in _REQUIRED_PRIORITY_ITEMS if item not in lower]
    assert not missing, (
        f"analyzer.md priority ordering is missing item(s): {missing!r}; "
        f"expected all 6: {list(_REQUIRED_PRIORITY_ITEMS)!r}"
    )


def test_analyzer_md_specifies_total_n_questions_footer(analyzer_md_text):
    """Design 0000045 Step 3.a: analyzer.md must specify the literal 'Total: N questions' footer."""
    assert "Total: N questions" in analyzer_md_text, (
        "analyzer.md must specify the literal output-format footer 'Total: N questions' "
        "(capital T, capital N) so the Director can verify the question list is complete"
    )


def test_skill_md_spawn_prompt_has_no_unescaped_curly_braces(skill_md_text):
    """Design 0000045 Step 3.b: spawn-prompt template must double any literal `{` / `}` per cafleet member-create rule.

    The cafleet member-create CLI runs every prompt through ``str.format()`` with
    ``session_id`` / ``agent_id`` / ``director_name`` / ``director_agent_id`` as kwargs.
    Any other literal brace must be doubled (``{{`` / ``}}``) or it will trip
    ``str.format`` at spawn time. The Director's instruction restricts the allowed
    unescaped placeholder to ``{agent_id}`` for this skill.
    """
    block = _spawn_prompt_block(skill_md_text)

    # Strip the only allowed unescaped placeholder, then collapse doubled braces.
    cleaned = block.replace("{agent_id}", "")
    cleaned = cleaned.replace("{{", "").replace("}}", "")

    leftover_open = cleaned.count("{")
    leftover_close = cleaned.count("}")

    assert leftover_open == 0 and leftover_close == 0, (
        "Step 2d spawn-prompt template contains unescaped curly braces that are not the "
        "permitted '{agent_id}' placeholder. cafleet member-create runs this through "
        "str.format(); literal braces must be doubled ('{{' / '}}'). "
        f"Leftover '{{' count: {leftover_open}, leftover '}}' count: {leftover_close}. "
        f"Block:\n{block}"
    )


# ---------------------------------------------------------------------------
# Step 4: Director-side helpers (CLI invocations + question.md schema)
# ---------------------------------------------------------------------------


import re  # noqa: E402  (kept here to scope to the Step 4 block)


_REQUIRED_CAFLEET_INVOCATIONS = (
    "cafleet session create",
    "cafleet member create",
    "cafleet message send",
    "cafleet message poll",
    "cafleet message ack",
    "cafleet member delete",
    "cafleet session delete",
)


def _question_md_format_section(skill_text: str) -> str:
    """Return the body under the question.md format heading, ignoring headings inside fenced blocks."""
    lines = skill_text.splitlines()
    start = None
    for index, line in enumerate(lines):
        stripped = line.lstrip()
        if (
            stripped.startswith("## ")
            and "question.md" in stripped.lower()
        ):
            start = index
            break
    assert start is not None, "SKILL.md missing the question.md format ## heading"
    end = len(lines)
    in_fence = False
    for index in range(start + 1, len(lines)):
        line = lines[index]
        stripped = line.lstrip()
        if stripped.startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        if stripped.startswith("## ") and "question.md" not in stripped.lower():
            end = index
            break
    return "\n".join(lines[start:end])


def _example_lines_for(text: str, verb: str) -> list[str]:
    """Return code-block lines (lines starting with ``cafleet``) that contain ``verb``.

    Skips prose-style mentions like inline backtick references — examples in this
    SKILL.md are line-leading ``cafleet`` invocations inside fenced code blocks.
    """
    matches = []
    for line in text.splitlines():
        if line.lstrip().startswith("cafleet ") and verb in line:
            matches.append(line)
    return matches


def _has_cafleet_invocation_line(text: str, verb_phrase: str) -> bool:
    """True if any line in ``text`` contains both ``cafleet`` and the verb phrase."""
    return any("cafleet" in line and verb_phrase in line for line in text.splitlines())


def test_skill_md_documents_all_required_cafleet_invocations(skill_md_text):
    """Design 0000045 Step 4.a: SKILL.md mentions every required Director-side cafleet invocation.

    The CLI carries flags like ``--session-id`` between ``cafleet`` and the verb,
    so a strict ``cafleet message poll`` substring would miss real examples.
    Match each invocation as ``cafleet`` + verb-phrase on the same line.
    """
    invocation_to_verb = {
        "cafleet session create": "session create",
        "cafleet member create": "member create",
        "cafleet message send": "message send",
        "cafleet message poll": "message poll",
        "cafleet message ack": "message ack",
        "cafleet member delete": "member delete",
        "cafleet session delete": "session delete",
    }
    missing = [
        invocation
        for invocation, verb in invocation_to_verb.items()
        if not _has_cafleet_invocation_line(skill_md_text, verb)
    ]
    assert not missing, (
        f"SKILL.md is missing required cafleet invocation(s): {missing!r}; "
        f"design 0000045 Step 4.a requires all 7 to be documented: "
        f"{list(_REQUIRED_CAFLEET_INVOCATIONS)!r}"
    )


def test_skill_md_session_create_example_uses_json_flag(skill_md_text):
    """Design 0000045 Step 4.a: cafleet session create example must use --json (Director parses agent_id)."""
    examples = _example_lines_for(skill_md_text, "session create")
    assert examples, (
        "SKILL.md has no line-leading 'cafleet ... session create' example "
        "(prose mentions in inline backticks do not count)"
    )
    assert any("--json" in line for line in examples), (
        f"At least one cafleet session create example must include --json so the Director can "
        f"parse the JSON response. Found examples: {examples!r}"
    )


def test_skill_md_member_create_example_uses_json_flag(skill_md_text):
    """Design 0000045 Step 4.a: cafleet member create example must use --json."""
    examples = _example_lines_for(skill_md_text, "member create")
    assert examples, (
        "SKILL.md has no line-leading 'cafleet ... member create' example "
        "(prose mentions in inline backticks do not count)"
    )
    assert any("--json" in line for line in examples), (
        f"At least one cafleet member create example must include --json so the Director can "
        f"parse the new member's agent_id. Found examples: {examples!r}"
    )


def test_skill_md_message_send_example_uses_to_and_text_flags(skill_md_text):
    """Design 0000045 Step 4.a: Director-to-Analyzer message-send example must show --to and --text."""
    send_lines = [
        line for line in skill_md_text.splitlines() if "cafleet" in line and "message send" in line
    ]
    assert send_lines, "SKILL.md has no 'cafleet ... message send' example"
    has_to_and_text = any("--to" in line and "--text" in line for line in send_lines)
    assert has_to_and_text, (
        "SKILL.md must show a cafleet message send example with both --to and --text flags. "
        f"Found send lines: {send_lines!r}"
    )


def test_skill_md_question_md_section_documents_interview_progress_marker(skill_md_text):
    """Design 0000045 Step 4.b: question.md schema includes the interview-progress HTML comment marker."""
    section = _question_md_format_section(skill_md_text)
    assert "<!-- interview-progress:" in section, (
        "question.md format section must show the literal '<!-- interview-progress:' "
        "HTML comment opener so readers see the marker is an HTML comment, not a heading"
    )
    assert "-->" in section, (
        "question.md format section must show the closing '-->' of the HTML comment marker"
    )
    assert "[" in section and "]" in section, (
        "interview-progress marker must contain a JSON array (square brackets) of section names"
    )


def test_skill_md_question_md_section_documents_questions_and_answers_headers(skill_md_text):
    """Design 0000045 Step 4.b: question.md schema documents ## Questions and ## Answers headers."""
    section = _question_md_format_section(skill_md_text)
    assert "## Questions" in section, (
        "question.md format section must document the literal '## Questions' header"
    )
    assert "## Answers" in section, (
        "question.md format section must document the literal '## Answers' header"
    )


def test_skill_md_question_md_section_documents_round_heading_format(skill_md_text):
    """Design 0000045 Step 4.b: round heading format with question-number range is documented."""
    section = _question_md_format_section(skill_md_text)
    assert "Round" in section, (
        "question.md Answers section example must include the literal text 'Round'"
    )
    range_match = re.search(r"Round\s+\d+.*\d+\s*-\s*\d+", section)
    assert range_match is not None, (
        "question.md Answers section example must show a 'Round X' heading with a question-number "
        "range like '(Questions 1-4)'. Got section:\n" + section
    )
