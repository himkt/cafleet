"""Tests for cafleet.coding_agent module.

Covers: CodingAgentConfig dataclass, CLAUDE/CODEX built-in configs,
CODING_AGENTS registry, get_coding_agent() helper.

Design doc 0000018 Step 2: CodingAgentConfig encapsulates agent-specific
details — binary name, extra args, default prompt template — and provides
build_command() and ensure_available() methods.
"""

import pytest


# ---------------------------------------------------------------------------
# CodingAgentConfig dataclass
# ---------------------------------------------------------------------------


class TestCodingAgentConfig:
    """Tests for the CodingAgentConfig dataclass itself."""

    def test_frozen_dataclass(self):
        """CodingAgentConfig instances are immutable (frozen=True)."""
        from cafleet.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="test-bin")
        with pytest.raises(AttributeError):
            config.name = "changed"

    def test_default_extra_args_is_empty_tuple(self):
        """extra_args defaults to an empty tuple when not provided."""
        from cafleet.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="test-bin")
        assert config.extra_args == ()

    def test_default_prompt_template_is_empty_string(self):
        """default_prompt_template defaults to empty string when not provided."""
        from cafleet.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="test-bin")
        assert config.default_prompt_template == ""

    def test_custom_fields(self):
        """All fields are stored correctly when explicitly provided."""
        from cafleet.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(
            name="custom",
            binary="custom-bin",
            extra_args=("--flag", "value"),
            default_prompt_template="Hello {director_name}",
        )
        assert config.name == "custom"
        assert config.binary == "custom-bin"
        assert config.extra_args == ("--flag", "value")
        assert config.default_prompt_template == "Hello {director_name}"

    def test_extra_args_is_immutable_tuple(self):
        """extra_args is a tuple, ensuring true immutability of config."""
        from cafleet.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="test-bin")
        assert isinstance(config.extra_args, tuple)


# ---------------------------------------------------------------------------
# build_command
# ---------------------------------------------------------------------------


class TestBuildCommand:
    """Tests for CodingAgentConfig.build_command()."""

    def test_no_extra_args(self):
        """With no extra_args, returns [binary, prompt]."""
        from cafleet.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="my-agent")
        result = config.build_command("do something")
        assert result == ["my-agent", "do something"]

    def test_with_extra_args(self):
        """With extra_args, returns [binary, *extra_args, prompt]."""
        from cafleet.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(
            name="test",
            binary="my-agent",
            extra_args=["--mode", "auto"],
        )
        result = config.build_command("do something")
        assert result == ["my-agent", "--mode", "auto", "do something"]

    def test_prompt_with_special_characters(self):
        """Prompt containing spaces and special chars is passed as a single element."""
        from cafleet.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="agent")
        prompt = (
            "Review PR #42 and post feedback. "
            "Use --agent-id 7ba91234-5678-90ab-cdef-112233445566."
        )
        result = config.build_command(prompt)
        assert result == ["agent", prompt]
        assert len(result) == 2

    def test_empty_prompt(self):
        """Empty string prompt is still appended as the last element."""
        from cafleet.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="agent")
        result = config.build_command("")
        assert result == ["agent", ""]

    def test_claude_build_command(self):
        """CLAUDE config produces [\"claude\", prompt] with no extra args."""
        from cafleet.coding_agent import CLAUDE

        result = CLAUDE.build_command("Hello world")
        assert result == ["claude", "Hello world"]

    def test_codex_build_command(self):
        """CODEX config produces [\"codex\", \"--approval-mode\", \"auto-edit\", prompt]."""
        from cafleet.coding_agent import CODEX

        result = CODEX.build_command("Hello world")
        assert result == ["codex", "--approval-mode", "auto-edit", "Hello world"]

    # -------------------------------------------------------------------------
    # Design doc 0000029 Step 4 — display_name kwarg plumbing
    # -------------------------------------------------------------------------

    def test_display_name_kwarg_injects_for_claude(self):
        """``CLAUDE.build_command("p", display_name="Drafter")`` produces the
        exact argv shape ``["claude", "--name", "Drafter", "p"]``.

        Design doc 0000029 §A: the display-name flag is injected between
        ``extra_args`` and the positional prompt — verified by the manual
        proof carried out on 2026-04-15.
        """
        from cafleet.coding_agent import CLAUDE

        result = CLAUDE.build_command("p", display_name="Drafter")
        assert result == ["claude", "--name", "Drafter", "p"], (
            f"CLAUDE must emit [binary, --name, <display_name>, <prompt>] "
            f"with the flag before the positional prompt. got: {result!r}"
        )

    def test_display_name_kwarg_no_op_for_codex(self):
        """``CODEX.build_command("p", display_name="Drafter")`` is byte-identical
        to the no-kwarg call — codex has no ``--name`` equivalent today, so
        ``display_name_args=()`` guards the injection.

        Design doc 0000029 §A table: ``CODEX.display_name_args = ()``.
        """
        from cafleet.coding_agent import CODEX

        result = CODEX.build_command("p", display_name="Drafter")
        assert result == ["codex", "--approval-mode", "auto-edit", "p"], (
            f"CODEX must ignore display_name (display_name_args=()). got: {result!r}"
        )

    def test_display_name_none_matches_default(self):
        """``display_name=None`` (or omitted) is byte-identical to not passing
        the kwarg — preserves backward compatibility with every existing
        positional caller.
        """
        from cafleet.coding_agent import CLAUDE, CODEX

        assert CLAUDE.build_command("p", display_name=None) == CLAUDE.build_command(
            "p"
        ), "CLAUDE: display_name=None must match no-kwarg call"
        assert CODEX.build_command("p", display_name=None) == CODEX.build_command(
            "p"
        ), "CODEX: display_name=None must match no-kwarg call"

    def test_display_name_with_spaces_preserved(self):
        """Whitespace inside ``display_name`` is preserved as a single list element.

        Design doc 0000029 §C: ``subprocess`` receives a ``list[str]`` so
        embedded spaces never get re-tokenised.
        """
        from cafleet.coding_agent import CLAUDE

        result = CLAUDE.build_command("p", display_name="Code Reviewer")
        assert result == ["claude", "--name", "Code Reviewer", "p"], (
            f"'Code Reviewer' must be one list element, not split on the space. "
            f"got: {result!r}"
        )

    def test_display_name_args_field_default_empty_tuple(self):
        """A ``CodingAgentConfig`` built without ``display_name_args`` exposes ``()``.

        Design doc 0000029 §A: the new field defaults to empty tuple so
        existing configs and tests don't need updating.
        """
        from cafleet.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="test-bin")
        assert config.display_name_args == (), (
            f"display_name_args must default to (). got: {config.display_name_args!r}"
        )
        assert isinstance(config.display_name_args, tuple), (
            f"display_name_args must be a tuple (frozen-dataclass immutability). "
            f"got: {type(config.display_name_args).__name__}"
        )


# ---------------------------------------------------------------------------
# ensure_available
# ---------------------------------------------------------------------------


class TestEnsureAvailable:
    """Tests for CodingAgentConfig.ensure_available()."""

    def test_raises_when_binary_not_found(self, monkeypatch):
        """Raises RuntimeError when shutil.which returns None."""
        from cafleet.coding_agent import CodingAgentConfig

        monkeypatch.setattr("shutil.which", lambda _: None)
        config = CodingAgentConfig(name="test", binary="nonexistent-bin")
        with pytest.raises(RuntimeError, match="'nonexistent-bin' binary not found"):
            config.ensure_available()

    def test_succeeds_when_binary_found(self, monkeypatch):
        """Does not raise when shutil.which returns a path."""
        from cafleet.coding_agent import CodingAgentConfig

        monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/test-bin")
        config = CodingAgentConfig(name="test", binary="test-bin")
        config.ensure_available()  # should not raise

    def test_checks_correct_binary_name(self, monkeypatch):
        """ensure_available() passes self.binary to shutil.which."""
        checked_binaries = []

        def mock_which(name):
            checked_binaries.append(name)
            return "/usr/bin/" + name

        monkeypatch.setattr("shutil.which", mock_which)

        from cafleet.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="specific-binary")
        config.ensure_available()
        assert checked_binaries == ["specific-binary"]

    def test_error_message_includes_binary_name(self, monkeypatch):
        """RuntimeError message includes the binary name for diagnostics."""
        monkeypatch.setattr("shutil.which", lambda _: None)

        from cafleet.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="my-special-agent")
        with pytest.raises(RuntimeError, match="my-special-agent"):
            config.ensure_available()


# ---------------------------------------------------------------------------
# Built-in configs: CLAUDE and CODEX
# ---------------------------------------------------------------------------


class TestClaudeConfig:
    """Tests for the CLAUDE built-in CodingAgentConfig constant."""

    def test_name(self):
        from cafleet.coding_agent import CLAUDE

        assert CLAUDE.name == "claude"

    def test_binary(self):
        from cafleet.coding_agent import CLAUDE

        assert CLAUDE.binary == "claude"

    def test_extra_args_empty(self):
        from cafleet.coding_agent import CLAUDE

        assert CLAUDE.extra_args == ()

    def test_prompt_template_contains_skill_reference(self):
        """Claude prompt template includes 'Load Skill(cafleet)' for skill loading."""
        from cafleet.coding_agent import CLAUDE

        assert "Load Skill(cafleet)" in CLAUDE.default_prompt_template

    def test_prompt_template_uses_format_placeholders_for_ids(self):
        """Claude prompt template uses {session_id}/{agent_id} format placeholders.

        Design doc 0000023: shell-expansion placeholders like $CAFLEET_AGENT_ID
        are replaced by Python ``str.format`` placeholders so the spawn site
        can bake literal UUIDs into the prompt text. The template must no
        longer reference the old shell-variable form.
        """
        from cafleet.coding_agent import CLAUDE

        assert "{session_id}" in CLAUDE.default_prompt_template
        assert "{agent_id}" in CLAUDE.default_prompt_template
        assert "$CAFLEET_AGENT_ID" not in CLAUDE.default_prompt_template
        assert "$CAFLEET_SESSION_ID" not in CLAUDE.default_prompt_template

    def test_prompt_template_has_format_placeholders(self):
        """Claude template accepts session_id/agent_id/director_name/director_agent_id."""
        from cafleet.coding_agent import CLAUDE

        # Verify template can be formatted with all four expected keys
        result = CLAUDE.default_prompt_template.format(
            session_id="550e8400-e29b-41d4-a716-446655440000",
            agent_id="7ba91234-5678-90ab-cdef-112233445566",
            director_name="Alice",
            director_agent_id="dir-001",
        )
        assert "550e8400-e29b-41d4-a716-446655440000" in result
        assert "7ba91234-5678-90ab-cdef-112233445566" in result
        assert "Alice" in result
        assert "dir-001" in result


class TestCodexConfig:
    """Tests for the CODEX built-in CodingAgentConfig constant."""

    def test_name(self):
        from cafleet.coding_agent import CODEX

        assert CODEX.name == "codex"

    def test_binary(self):
        from cafleet.coding_agent import CODEX

        assert CODEX.binary == "codex"

    def test_extra_args(self):
        """Codex config includes --approval-mode auto-edit flags."""
        from cafleet.coding_agent import CODEX

        assert CODEX.extra_args == ("--approval-mode", "auto-edit")

    def test_prompt_template_no_skill_reference(self):
        """Codex prompt template does NOT include 'Skill(' — Codex has no skill mechanism."""
        from cafleet.coding_agent import CODEX

        assert "Skill(" not in CODEX.default_prompt_template

    def test_prompt_template_uses_format_placeholders_for_ids(self):
        """Codex prompt template uses {session_id}/{agent_id} format placeholders.

        Design doc 0000023: same migration as CLAUDE — shell-variable form
        (``$CAFLEET_AGENT_ID``) is replaced by ``str.format`` placeholders.
        """
        from cafleet.coding_agent import CODEX

        assert "{session_id}" in CODEX.default_prompt_template
        assert "{agent_id}" in CODEX.default_prompt_template
        assert "$CAFLEET_AGENT_ID" not in CODEX.default_prompt_template
        assert "$CAFLEET_SESSION_ID" not in CODEX.default_prompt_template

    def test_prompt_template_has_format_placeholders(self):
        """Codex template accepts session_id/agent_id/director_name/director_agent_id."""
        from cafleet.coding_agent import CODEX

        result = CODEX.default_prompt_template.format(
            session_id="550e8400-e29b-41d4-a716-446655440000",
            agent_id="7ba91234-5678-90ab-cdef-112233445566",
            director_name="Bob",
            director_agent_id="dir-002",
        )
        assert "550e8400-e29b-41d4-a716-446655440000" in result
        assert "7ba91234-5678-90ab-cdef-112233445566" in result
        assert "Bob" in result
        assert "dir-002" in result

    def test_prompt_template_contains_explicit_cli_instructions(self):
        """Codex template includes explicit cafleet CLI usage (poll, ack, send).

        Design doc 0000023 (Copilot review fix): ``--session-id`` is a
        root-group global option (before the subcommand); ``--agent-id`` is
        a per-subcommand option (after the subcommand). The template must
        emit the exact literal form click accepts so that Claude Code's
        ``permissions.allow`` can match it as a literal string.
        """
        from cafleet.coding_agent import CODEX

        template = CODEX.default_prompt_template
        assert (
            "cafleet --session-id {session_id} poll --agent-id {agent_id}" in template
        )
        assert "cafleet --session-id {session_id} ack --agent-id {agent_id}" in template
        assert (
            "cafleet --session-id {session_id} send --agent-id {agent_id}" in template
        )


# ---------------------------------------------------------------------------
# CODING_AGENTS registry
# ---------------------------------------------------------------------------


class TestCodingAgentsRegistry:
    """Tests for the CODING_AGENTS dict registry."""

    def test_contains_claude(self):
        from cafleet.coding_agent import CLAUDE, CODING_AGENTS

        assert "claude" in CODING_AGENTS
        assert CODING_AGENTS["claude"] is CLAUDE

    def test_contains_codex(self):
        from cafleet.coding_agent import CODEX, CODING_AGENTS

        assert "codex" in CODING_AGENTS
        assert CODING_AGENTS["codex"] is CODEX

    def test_exactly_two_entries(self):
        """Registry has exactly claude and codex — no unexpected entries."""
        from cafleet.coding_agent import CODING_AGENTS

        assert set(CODING_AGENTS.keys()) == {"claude", "codex"}


# ---------------------------------------------------------------------------
# get_coding_agent
# ---------------------------------------------------------------------------


class TestGetCodingAgent:
    """Tests for the get_coding_agent() helper function."""

    def test_returns_claude_config(self):
        from cafleet.coding_agent import CLAUDE, get_coding_agent

        result = get_coding_agent("claude")
        assert result is CLAUDE

    def test_returns_codex_config(self):
        from cafleet.coding_agent import CODEX, get_coding_agent

        result = get_coding_agent("codex")
        assert result is CODEX

    def test_raises_valueerror_for_unknown_name(self):
        """Raises ValueError for a name not in CODING_AGENTS."""
        from cafleet.coding_agent import get_coding_agent

        with pytest.raises(ValueError):
            get_coding_agent("unknown-agent")

    def test_raises_valueerror_for_empty_string(self):
        """Raises ValueError for an empty string."""
        from cafleet.coding_agent import get_coding_agent

        with pytest.raises(ValueError):
            get_coding_agent("")

    def test_error_message_includes_unknown_name(self):
        """ValueError message includes the invalid name for diagnostics."""
        from cafleet.coding_agent import get_coding_agent

        with pytest.raises(ValueError, match="aider"):
            get_coding_agent("aider")
