import pytest

from cafleet.coding_agent import (
    CLAUDE,
    CODEX,
    CODING_AGENTS,
    CodingAgentConfig,
    get_coding_agent,
)


class TestCodingAgentConfig:
    def test_frozen_dataclass(self):
        config = CodingAgentConfig(name="test", binary="test-bin")
        with pytest.raises(AttributeError):
            config.name = "changed"

    def test_default_extra_args_is_empty_tuple(self):
        config = CodingAgentConfig(name="test", binary="test-bin")
        assert config.extra_args == ()

    def test_default_prompt_template_is_empty_string(self):
        config = CodingAgentConfig(name="test", binary="test-bin")
        assert config.default_prompt_template == ""

    def test_custom_fields(self):
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
        config = CodingAgentConfig(name="test", binary="test-bin")
        assert isinstance(config.extra_args, tuple)


class TestBuildCommand:
    def test_no_extra_args(self):
        config = CodingAgentConfig(name="test", binary="my-agent")
        result = config.build_command("do something")
        assert result == ["my-agent", "do something"]

    def test_with_extra_args(self):
        config = CodingAgentConfig(
            name="test",
            binary="my-agent",
            extra_args=["--mode", "auto"],
        )
        result = config.build_command("do something")
        assert result == ["my-agent", "--mode", "auto", "do something"]

    def test_prompt_with_special_characters(self):
        config = CodingAgentConfig(name="test", binary="agent")
        prompt = (
            "Review PR #42 and post feedback. "
            "Use --agent-id 7ba91234-5678-90ab-cdef-112233445566."
        )
        result = config.build_command(prompt)
        assert result == ["agent", prompt]
        assert len(result) == 2

    def test_empty_prompt(self):
        config = CodingAgentConfig(name="test", binary="agent")
        result = config.build_command("")
        assert result == ["agent", ""]

    def test_claude_build_command(self):
        result = CLAUDE.build_command("Hello world")
        assert result == ["claude", "Hello world"]

    def test_codex_build_command(self):
        result = CODEX.build_command("Hello world")
        assert result == ["codex", "--approval-mode", "auto-edit", "Hello world"]

    def test_display_name_kwarg_injects_for_claude(self):
        result = CLAUDE.build_command("p", display_name="Drafter")
        assert result == ["claude", "--name", "Drafter", "p"]

    def test_display_name_kwarg_no_op_for_codex(self):
        result = CODEX.build_command("p", display_name="Drafter")
        assert result == ["codex", "--approval-mode", "auto-edit", "p"]

    def test_display_name_none_matches_default(self):
        assert CLAUDE.build_command("p", display_name=None) == CLAUDE.build_command("p")
        assert CODEX.build_command("p", display_name=None) == CODEX.build_command("p")

    def test_display_name_with_spaces_preserved(self):
        result = CLAUDE.build_command("p", display_name="Code Reviewer")
        assert result == ["claude", "--name", "Code Reviewer", "p"]

    def test_display_name_args_field_default_empty_tuple(self):
        config = CodingAgentConfig(name="test", binary="test-bin")
        assert config.display_name_args == ()
        assert isinstance(config.display_name_args, tuple)


class TestEnsureAvailable:
    def test_raises_when_binary_not_found(self, monkeypatch):
        monkeypatch.setattr("shutil.which", lambda _: None)
        config = CodingAgentConfig(name="test", binary="nonexistent-bin")
        with pytest.raises(RuntimeError, match="'nonexistent-bin' binary not found"):
            config.ensure_available()

    def test_succeeds_when_binary_found(self, monkeypatch):
        monkeypatch.setattr("shutil.which", lambda _: "/usr/bin/test-bin")
        config = CodingAgentConfig(name="test", binary="test-bin")
        config.ensure_available()

    def test_checks_correct_binary_name(self, monkeypatch):
        checked_binaries = []

        def mock_which(name):
            checked_binaries.append(name)
            return "/usr/bin/" + name

        monkeypatch.setattr("shutil.which", mock_which)

        config = CodingAgentConfig(name="test", binary="specific-binary")
        config.ensure_available()
        assert checked_binaries == ["specific-binary"]

    def test_error_message_includes_binary_name(self, monkeypatch):
        monkeypatch.setattr("shutil.which", lambda _: None)
        config = CodingAgentConfig(name="test", binary="my-special-agent")
        with pytest.raises(RuntimeError, match="my-special-agent"):
            config.ensure_available()


class TestClaudeConfig:
    def test_name(self):
        assert CLAUDE.name == "claude"

    def test_binary(self):
        assert CLAUDE.binary == "claude"

    def test_extra_args_empty(self):
        assert CLAUDE.extra_args == ()

    def test_prompt_template_contains_skill_reference(self):
        assert "Load Skill(cafleet)" in CLAUDE.default_prompt_template

    def test_prompt_template_uses_format_placeholders_for_ids(self):
        assert "{session_id}" in CLAUDE.default_prompt_template
        assert "{agent_id}" in CLAUDE.default_prompt_template
        assert "$CAFLEET_AGENT_ID" not in CLAUDE.default_prompt_template
        assert "$CAFLEET_SESSION_ID" not in CLAUDE.default_prompt_template

    def test_prompt_template_has_format_placeholders(self):
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
    def test_name(self):
        assert CODEX.name == "codex"

    def test_binary(self):
        assert CODEX.binary == "codex"

    def test_extra_args(self):
        assert CODEX.extra_args == ("--approval-mode", "auto-edit")

    def test_prompt_template_no_skill_reference(self):
        assert "Skill(" not in CODEX.default_prompt_template

    def test_prompt_template_uses_format_placeholders_for_ids(self):
        assert "{session_id}" in CODEX.default_prompt_template
        assert "{agent_id}" in CODEX.default_prompt_template
        assert "$CAFLEET_AGENT_ID" not in CODEX.default_prompt_template
        assert "$CAFLEET_SESSION_ID" not in CODEX.default_prompt_template

    def test_prompt_template_has_format_placeholders(self):
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
        template = CODEX.default_prompt_template
        assert (
            "cafleet --session-id {session_id} poll --agent-id {agent_id}" in template
        )
        assert "cafleet --session-id {session_id} ack --agent-id {agent_id}" in template
        assert (
            "cafleet --session-id {session_id} send --agent-id {agent_id}" in template
        )


class TestCodingAgentsRegistry:
    def test_contains_claude(self):
        assert "claude" in CODING_AGENTS
        assert CODING_AGENTS["claude"] is CLAUDE

    def test_contains_codex(self):
        assert "codex" in CODING_AGENTS
        assert CODING_AGENTS["codex"] is CODEX

    def test_exactly_two_entries(self):
        assert set(CODING_AGENTS.keys()) == {"claude", "codex"}


class TestGetCodingAgent:
    def test_returns_claude_config(self):
        result = get_coding_agent("claude")
        assert result is CLAUDE

    def test_returns_codex_config(self):
        result = get_coding_agent("codex")
        assert result is CODEX

    def test_raises_valueerror_for_unknown_name(self):
        with pytest.raises(ValueError, match="Unknown coding agent"):
            get_coding_agent("unknown-agent")

    def test_raises_valueerror_for_empty_string(self):
        with pytest.raises(ValueError, match="Unknown coding agent"):
            get_coding_agent("")

    def test_error_message_includes_unknown_name(self):
        with pytest.raises(ValueError, match="aider"):
            get_coding_agent("aider")
