"""Tests for hikyaku.coding_agent module.

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
        from hikyaku.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="test-bin")
        with pytest.raises(AttributeError):
            config.name = "changed"

    def test_default_extra_args_is_empty_list(self):
        """extra_args defaults to an empty list when not provided."""
        from hikyaku.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="test-bin")
        assert config.extra_args == []

    def test_default_prompt_template_is_empty_string(self):
        """default_prompt_template defaults to empty string when not provided."""
        from hikyaku.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="test-bin")
        assert config.default_prompt_template == ""

    def test_custom_fields(self):
        """All fields are stored correctly when explicitly provided."""
        from hikyaku.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(
            name="custom",
            binary="custom-bin",
            extra_args=["--flag", "value"],
            default_prompt_template="Hello {director_name}",
        )
        assert config.name == "custom"
        assert config.binary == "custom-bin"
        assert config.extra_args == ["--flag", "value"]
        assert config.default_prompt_template == "Hello {director_name}"

    def test_extra_args_default_factory_isolation(self):
        """Each instance gets its own default list (field(default_factory=list))."""
        from hikyaku.coding_agent import CodingAgentConfig

        a = CodingAgentConfig(name="a", binary="a")
        b = CodingAgentConfig(name="b", binary="b")
        assert a.extra_args is not b.extra_args


# ---------------------------------------------------------------------------
# build_command
# ---------------------------------------------------------------------------


class TestBuildCommand:
    """Tests for CodingAgentConfig.build_command()."""

    def test_no_extra_args(self):
        """With no extra_args, returns [binary, prompt]."""
        from hikyaku.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="my-agent")
        result = config.build_command("do something")
        assert result == ["my-agent", "do something"]

    def test_with_extra_args(self):
        """With extra_args, returns [binary, *extra_args, prompt]."""
        from hikyaku.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(
            name="test",
            binary="my-agent",
            extra_args=["--mode", "auto"],
        )
        result = config.build_command("do something")
        assert result == ["my-agent", "--mode", "auto", "do something"]

    def test_prompt_with_special_characters(self):
        """Prompt containing spaces and special chars is passed as a single element."""
        from hikyaku.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="agent")
        prompt = "Review PR #42 and post feedback. Use $HIKYAKU_AGENT_ID."
        result = config.build_command(prompt)
        assert result == ["agent", prompt]
        assert len(result) == 2

    def test_empty_prompt(self):
        """Empty string prompt is still appended as the last element."""
        from hikyaku.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="agent")
        result = config.build_command("")
        assert result == ["agent", ""]

    def test_claude_build_command(self):
        """CLAUDE config produces [\"claude\", prompt] with no extra args."""
        from hikyaku.coding_agent import CLAUDE

        result = CLAUDE.build_command("Hello world")
        assert result == ["claude", "Hello world"]

    def test_codex_build_command(self):
        """CODEX config produces [\"codex\", \"--approval-mode\", \"auto-edit\", prompt]."""
        from hikyaku.coding_agent import CODEX

        result = CODEX.build_command("Hello world")
        assert result == ["codex", "--approval-mode", "auto-edit", "Hello world"]


# ---------------------------------------------------------------------------
# ensure_available
# ---------------------------------------------------------------------------


class TestEnsureAvailable:
    """Tests for CodingAgentConfig.ensure_available()."""

    def test_raises_when_binary_not_found(self, monkeypatch):
        """Raises RuntimeError when shutil.which returns None."""
        from hikyaku.coding_agent import CodingAgentConfig

        monkeypatch.setattr("shutil.which", lambda _: None)
        config = CodingAgentConfig(name="test", binary="nonexistent-bin")
        with pytest.raises(RuntimeError, match="'nonexistent-bin' binary not found"):
            config.ensure_available()

    def test_succeeds_when_binary_found(self, monkeypatch):
        """Does not raise when shutil.which returns a path."""
        from hikyaku.coding_agent import CodingAgentConfig

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

        from hikyaku.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="specific-binary")
        config.ensure_available()
        assert checked_binaries == ["specific-binary"]

    def test_error_message_includes_binary_name(self, monkeypatch):
        """RuntimeError message includes the binary name for diagnostics."""
        monkeypatch.setattr("shutil.which", lambda _: None)

        from hikyaku.coding_agent import CodingAgentConfig

        config = CodingAgentConfig(name="test", binary="my-special-agent")
        with pytest.raises(RuntimeError, match="my-special-agent"):
            config.ensure_available()


# ---------------------------------------------------------------------------
# Built-in configs: CLAUDE and CODEX
# ---------------------------------------------------------------------------


class TestClaudeConfig:
    """Tests for the CLAUDE built-in CodingAgentConfig constant."""

    def test_name(self):
        from hikyaku.coding_agent import CLAUDE

        assert CLAUDE.name == "claude"

    def test_binary(self):
        from hikyaku.coding_agent import CLAUDE

        assert CLAUDE.binary == "claude"

    def test_extra_args_empty(self):
        from hikyaku.coding_agent import CLAUDE

        assert CLAUDE.extra_args == []

    def test_prompt_template_contains_skill_reference(self):
        """Claude prompt template includes 'Load Skill(hikyaku)' for skill loading."""
        from hikyaku.coding_agent import CLAUDE

        assert "Load Skill(hikyaku)" in CLAUDE.default_prompt_template

    def test_prompt_template_contains_agent_id_placeholder(self):
        """Claude prompt template references $HIKYAKU_AGENT_ID."""
        from hikyaku.coding_agent import CLAUDE

        assert "$HIKYAKU_AGENT_ID" in CLAUDE.default_prompt_template

    def test_prompt_template_has_format_placeholders(self):
        """Claude prompt template has {director_name} and {director_agent_id} placeholders."""
        from hikyaku.coding_agent import CLAUDE

        # Verify template can be formatted with the expected keys
        result = CLAUDE.default_prompt_template.format(
            director_name="Alice",
            director_agent_id="dir-001",
        )
        assert "Alice" in result
        assert "dir-001" in result


class TestCodexConfig:
    """Tests for the CODEX built-in CodingAgentConfig constant."""

    def test_name(self):
        from hikyaku.coding_agent import CODEX

        assert CODEX.name == "codex"

    def test_binary(self):
        from hikyaku.coding_agent import CODEX

        assert CODEX.binary == "codex"

    def test_extra_args(self):
        """Codex config includes --approval-mode auto-edit flags."""
        from hikyaku.coding_agent import CODEX

        assert CODEX.extra_args == ["--approval-mode", "auto-edit"]

    def test_prompt_template_no_skill_reference(self):
        """Codex prompt template does NOT include 'Skill(' — Codex has no skill mechanism."""
        from hikyaku.coding_agent import CODEX

        assert "Skill(" not in CODEX.default_prompt_template

    def test_prompt_template_contains_agent_id_placeholder(self):
        """Codex prompt template references $HIKYAKU_AGENT_ID."""
        from hikyaku.coding_agent import CODEX

        assert "$HIKYAKU_AGENT_ID" in CODEX.default_prompt_template

    def test_prompt_template_has_format_placeholders(self):
        """Codex prompt template has {director_name} and {director_agent_id} placeholders."""
        from hikyaku.coding_agent import CODEX

        result = CODEX.default_prompt_template.format(
            director_name="Bob",
            director_agent_id="dir-002",
        )
        assert "Bob" in result
        assert "dir-002" in result

    def test_prompt_template_contains_explicit_cli_instructions(self):
        """Codex template includes explicit hikyaku CLI usage (poll, ack, send)."""
        from hikyaku.coding_agent import CODEX

        template = CODEX.default_prompt_template
        assert "hikyaku poll" in template
        assert "hikyaku ack" in template
        assert "hikyaku send" in template


# ---------------------------------------------------------------------------
# CODING_AGENTS registry
# ---------------------------------------------------------------------------


class TestCodingAgentsRegistry:
    """Tests for the CODING_AGENTS dict registry."""

    def test_contains_claude(self):
        from hikyaku.coding_agent import CLAUDE, CODING_AGENTS

        assert "claude" in CODING_AGENTS
        assert CODING_AGENTS["claude"] is CLAUDE

    def test_contains_codex(self):
        from hikyaku.coding_agent import CODEX, CODING_AGENTS

        assert "codex" in CODING_AGENTS
        assert CODING_AGENTS["codex"] is CODEX

    def test_exactly_two_entries(self):
        """Registry has exactly claude and codex — no unexpected entries."""
        from hikyaku.coding_agent import CODING_AGENTS

        assert set(CODING_AGENTS.keys()) == {"claude", "codex"}


# ---------------------------------------------------------------------------
# get_coding_agent
# ---------------------------------------------------------------------------


class TestGetCodingAgent:
    """Tests for the get_coding_agent() helper function."""

    def test_returns_claude_config(self):
        from hikyaku.coding_agent import CLAUDE, get_coding_agent

        result = get_coding_agent("claude")
        assert result is CLAUDE

    def test_returns_codex_config(self):
        from hikyaku.coding_agent import CODEX, get_coding_agent

        result = get_coding_agent("codex")
        assert result is CODEX

    def test_raises_valueerror_for_unknown_name(self):
        """Raises ValueError for a name not in CODING_AGENTS."""
        from hikyaku.coding_agent import get_coding_agent

        with pytest.raises(ValueError):
            get_coding_agent("unknown-agent")

    def test_raises_valueerror_for_empty_string(self):
        """Raises ValueError for an empty string."""
        from hikyaku.coding_agent import get_coding_agent

        with pytest.raises(ValueError):
            get_coding_agent("")

    def test_error_message_includes_unknown_name(self):
        """ValueError message includes the invalid name for diagnostics."""
        from hikyaku.coding_agent import get_coding_agent

        with pytest.raises(ValueError, match="aider"):
            get_coding_agent("aider")
