import pytest

from cafleet.coding_agent import CLAUDE, CodingAgentConfig


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
        assert result == [
            "claude",
            "--permission-mode",
            "dontAsk",
            "Hello world",
        ]

    def test_display_name_kwarg_injects_for_claude(self):
        result = CLAUDE.build_command("p", display_name="Drafter")
        assert result == [
            "claude",
            "--permission-mode",
            "dontAsk",
            "--name",
            "Drafter",
            "p",
        ]

    def test_display_name_none_matches_default(self):
        assert CLAUDE.build_command("p", display_name=None) == CLAUDE.build_command("p")

    def test_display_name_with_spaces_preserved(self):
        result = CLAUDE.build_command("p", display_name="Code Reviewer")
        assert result == [
            "claude",
            "--permission-mode",
            "dontAsk",
            "--name",
            "Code Reviewer",
            "p",
        ]

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


class TestPermissionArgs:
    """``permission_args`` field — always-injected spawn argv.

    Pinned argv ordering: ``[binary, *extra_args, *permission_args, *name_args, prompt]``
    — ``permission_args`` MUST come BEFORE ``name_args``. Asserted by index,
    not just membership, so a future revert that flips the order would fail loudly.
    """

    def test_claude_permission_args_constant(self):
        assert CLAUDE.permission_args == ("--permission-mode", "dontAsk")

    def test_permission_args_field_default_empty_tuple(self):
        config = CodingAgentConfig(name="test", binary="test-bin")
        assert config.permission_args == ()
        assert isinstance(config.permission_args, tuple)

    def test_claude_build_command_no_display_name(self):
        result = CLAUDE.build_command("hello")
        assert result == ["claude", "--permission-mode", "dontAsk", "hello"]

    def test_claude_build_command_with_display_name(self):
        result = CLAUDE.build_command("hello", display_name="Drafter")
        assert result == [
            "claude",
            "--permission-mode",
            "dontAsk",
            "--name",
            "Drafter",
            "hello",
        ]

    def test_permission_args_appear_before_name_args(self):
        result = CLAUDE.build_command("p", display_name="Drafter")
        perm_index = result.index("--permission-mode")
        name_index = result.index("--name")
        assert perm_index < name_index, (
            f"permission_args must precede name_args; got {result!r}"
        )

    def test_custom_config_with_permission_args_injects_tokens(self):
        config = CodingAgentConfig(
            name="custom",
            binary="custom-bin",
            extra_args=("--mode", "auto"),
            permission_args=("--allowedTools", "Read"),
        )
        result = config.build_command("p")
        assert result == ["custom-bin", "--mode", "auto", "--allowedTools", "Read", "p"]

    def test_custom_config_with_empty_permission_args_no_op(self):
        config = CodingAgentConfig(
            name="custom",
            binary="custom-bin",
            extra_args=("--mode", "auto"),
            permission_args=(),
        )
        result = config.build_command("p")
        assert result == ["custom-bin", "--mode", "auto", "p"]


class TestPromptTemplates:
    """dontAsk wiring reminder in the claude template."""

    _STANDARD_KWARGS = {
        "session_id": "550e8400-e29b-41d4-a716-446655440000",
        "agent_id": "7ba91234-5678-90ab-cdef-112233445566",
        "director_name": "Alice",
        "director_agent_id": "dir-001",
    }

    def test_claude_template_documents_dontask_mode(self):
        """The template tells the spawned member its harness runs in
        ``dontAsk`` mode and the Bash tool is enabled. The canary
        ``"dontAsk"`` is the smallest change-resistant marker.
        """
        assert "dontAsk" in CLAUDE.default_prompt_template

    def test_claude_template_format_succeeds_with_standard_kwargs(self):
        result = CLAUDE.default_prompt_template.format(**self._STANDARD_KWARGS)
        assert "550e8400-e29b-41d4-a716-446655440000" in result
        assert "7ba91234-5678-90ab-cdef-112233445566" in result
        assert "Alice" in result
        assert "dir-001" in result
