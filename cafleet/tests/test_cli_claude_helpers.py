"""Tests for the inlined claude-spawn helpers in ``cli.py`` (design 0000041 §B).

The dataclass abstraction (``CodingAgentConfig`` + ``CLAUDE`` singleton) is
inlined into ``cafleet/src/cafleet/cli.py`` as four module-level symbols:

- ``_CLAUDE_BINARY = "claude"``
- ``_CLAUDE_PROMPT_TEMPLATE`` — multi-line ``str.format`` template with
  ``{session_id}``, ``{agent_id}``, ``{director_name}``, ``{director_agent_id}``.
- ``_build_claude_command(prompt, *, display_name)`` — returns the spawn argv.
- ``_ensure_claude_available()`` — ``RuntimeError`` if the binary is missing.
"""

import pytest

from cafleet.cli import (
    _CLAUDE_PROMPT_TEMPLATE,
    _build_claude_command,
    _ensure_claude_available,
)


class TestBuildClaudeCommand:
    def test_argv_shape(self):
        assert _build_claude_command("PROMPT_TEXT", display_name="Bob") == [
            "claude",
            "--permission-mode",
            "dontAsk",
            "--name",
            "Bob",
            "PROMPT_TEXT",
        ]

    def test_preserves_prompt_with_special_chars(self):
        prompt = (
            "Review PR #42.\n"
            "Use --agent-id 7ba91234-5678-90ab-cdef-112233445566.\n"
            'Quote: "hello" and {literal_braces}.'
        )
        result = _build_claude_command(prompt, display_name="Drafter")
        assert result[-1] == prompt
        assert result[0] == "claude"

    def test_preserves_display_name_with_spaces(self):
        result = _build_claude_command("p", display_name="Code Reviewer")
        assert result == [
            "claude",
            "--permission-mode",
            "dontAsk",
            "--name",
            "Code Reviewer",
            "p",
        ]


class TestEnsureClaudeAvailable:
    def test_raises_when_missing(self, monkeypatch):
        monkeypatch.setattr("cafleet.cli.shutil.which", lambda _: None)
        with pytest.raises(RuntimeError, match="claude"):
            _ensure_claude_available()

    def test_silent_when_found(self, monkeypatch):
        monkeypatch.setattr("cafleet.cli.shutil.which", lambda _: "/usr/bin/claude")
        assert _ensure_claude_available() is None


class TestClaudePromptTemplate:
    _STANDARD_KWARGS = {
        "session_id": "550e8400-e29b-41d4-a716-446655440000",
        "agent_id": "7ba91234-5678-90ab-cdef-112233445566",
        "director_name": "Alice",
        "director_agent_id": "dir-001",
    }

    def test_has_required_placeholders_and_markers(self):
        assert "{session_id}" in _CLAUDE_PROMPT_TEMPLATE
        assert "{agent_id}" in _CLAUDE_PROMPT_TEMPLATE
        assert "{director_name}" in _CLAUDE_PROMPT_TEMPLATE
        assert "{director_agent_id}" in _CLAUDE_PROMPT_TEMPLATE
        assert "Load Skill(cafleet)" in _CLAUDE_PROMPT_TEMPLATE
        assert "dontAsk" in _CLAUDE_PROMPT_TEMPLATE

    def test_format_succeeds_with_standard_kwargs(self):
        result = _CLAUDE_PROMPT_TEMPLATE.format(**self._STANDARD_KWARGS)
        assert "550e8400-e29b-41d4-a716-446655440000" in result
        assert "7ba91234-5678-90ab-cdef-112233445566" in result
        assert "Alice" in result
        assert "dir-001" in result
